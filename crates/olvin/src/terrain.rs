//! Terrains in scenes (feature `terrain`): an object with
//! `EntityDesc::terrain` gets one child entity per chunk (levels of detail
//! included) and, with a Mesh Collider, the terrain's triangles. The editor
//! builds the same parts for its view through [`TerrainParts`].

use std::path::Path;

pub use olvin_terrain::*;

use glam::Vec3;
use olvin_render::{MaterialHandle, Meshes, ShaderHandle, TextureHandle};

use crate::render_assets::{Registries, RenderAssets};

/// The data of `rel` under `root`, or a flat terrain when there is none
/// (a new terrain, or a missing file: logged).
pub fn load_data(root: Option<&Path>, rel: &str) -> TerrainData {
    if rel.is_empty() {
        return TerrainData::flat(DEFAULT_RESOLUTION);
    }
    let Some(root) = root else {
        return TerrainData::flat(DEFAULT_RESOLUTION);
    };
    match TerrainData::load(olvin_assets::AssetRoot::new(root).resolve(rel)) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("terrain: {e}; using a flat one");
            TerrainData::flat(DEFAULT_RESOLUTION)
        }
    }
}

/// The GPU side of one terrain: chunk meshes and the blended material.
pub struct TerrainParts {
    pub chunks: Vec<mesh::ChunkMeshes>,
    pub material: Option<MaterialHandle>,
    atlas: Option<TextureHandle>,
    weights: Option<TextureHandle>,
    shader: Option<ShaderHandle>,
    /// What the atlas and shader were made from.
    look: (Vec<TerrainLayer>, u32),
    size: Vec3,
}

impl TerrainParts {
    pub fn build(
        data: &TerrainData,
        desc: &TerrainDesc,
        meshes: &mut Meshes,
        assets: &mut RenderAssets,
        r: &mut Registries,
    ) -> Self {
        let mut parts = Self {
            chunks: mesh::add_chunks(meshes, data, desc.size),
            material: None,
            atlas: None,
            weights: None,
            shader: None,
            look: (Vec::new(), 0),
            size: desc.size,
        };
        parts.rebuild_look(data, desc, assets, r);
        parts
    }

    /// Follows `desc` and `data`: layers (atlas and shader), size (every
    /// chunk), the resolution of a replaced data set.
    pub fn sync_desc(
        &mut self,
        data: &TerrainData,
        desc: &TerrainDesc,
        meshes: &mut Meshes,
        assets: &mut RenderAssets,
        r: &mut Registries,
    ) {
        let same_grid = self.chunks.len() == mesh::chunks(data).len();
        if desc.size != self.size || !same_grid {
            if same_grid {
                let last = data.resolution() - 1;
                mesh::update_chunks(
                    meshes,
                    &self.chunks,
                    data,
                    desc.size,
                    ((0, 0), (last, last)),
                );
            } else {
                self.chunks = mesh::add_chunks(meshes, data, desc.size);
            }
            self.size = desc.size;
        }
        let layers: Vec<TerrainLayer> =
            desc.layers.iter().take(look::MAX_LAYERS).cloned().collect();
        if self.look != (layers, data.resolution()) {
            self.rebuild_look(data, desc, assets, r);
        }
    }

    /// Chunks under the vertex rectangle changed height.
    pub fn update_heights(
        &mut self,
        meshes: &mut Meshes,
        data: &TerrainData,
        rect: ((u32, u32), (u32, u32)),
    ) {
        mesh::update_chunks(meshes, &self.chunks, data, self.size, rect);
    }

    /// The layer weights changed.
    pub fn update_weights(&mut self, textures: &mut olvin_render::Textures, data: &TerrainData) {
        if let Some(h) = self.weights {
            textures.set(h, look::weight_map(data));
        }
    }

    fn rebuild_look(
        &mut self,
        data: &TerrainData,
        desc: &TerrainDesc,
        assets: &mut RenderAssets,
        r: &mut Registries,
    ) {
        let layers: Vec<TerrainLayer> =
            desc.layers.iter().take(look::MAX_LAYERS).cloned().collect();
        let atlas = look::atlas(&layers, |p| assets.image(p, true));
        let weights = look::weight_map(data);
        let source = look::surface_shader(&layers, data.resolution());
        let atlas = match self.atlas {
            Some(h) => {
                r.textures.set(h, atlas);
                h
            }
            None => r.textures.add(atlas),
        };
        let weights = match self.weights {
            Some(h) => {
                r.textures.set(h, weights);
                h
            }
            None => r.textures.add(weights),
        };
        let shader = match self.shader {
            Some(h) => {
                r.shaders.set(h, source);
                h
            }
            None => r.shaders.add(source),
        };
        if let Some(e) = r.shaders.error(shader) {
            log::warn!("terrain shader: {e}");
        }
        let material = look::material(atlas, weights, shader);
        self.material = Some(match self.material {
            Some(h) => {
                r.materials.set(h, material);
                h
            }
            None => r.materials.add(material),
        });
        (self.atlas, self.weights, self.shader) = (Some(atlas), Some(weights), Some(shader));
        self.look = (layers, data.resolution());
    }
}

/// Builds the terrain of a scene object: its chunks and material, and its
/// collision triangles when `collide`. `None` without the render registries.
pub(crate) fn spawn_parts(
    res: &olvin_core::Resources,
    meshes: &mut Meshes,
    desc: &TerrainDesc,
    collide: bool,
) -> Option<(TerrainParts, Option<olvin_scene::MeshGeometry>)> {
    let mut assets = res.get_mut::<RenderAssets>()?;
    let mut textures = res.get_mut::<olvin_render::Textures>()?;
    let mut materials = res.get_mut::<olvin_render::Materials>()?;
    let mut shaders = res.get_mut::<olvin_render::Shaders>()?;
    let data = load_data(assets.root(), &desc.data);
    let parts = TerrainParts::build(
        &data,
        desc,
        meshes,
        &mut assets,
        &mut Registries {
            textures: &mut textures,
            materials: &mut materials,
            shaders: &mut shaders,
        },
    );
    Some((parts, collide.then(|| mesh::collision(&data, desc.size))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_file::spawn;
    use olvin_core::Resources;
    use olvin_ecs::World;
    use olvin_render::{MeshRenderer, RenderSettings};
    use olvin_scene::MeshGeometry;
    use olvin_scene::file::{ColliderDesc, ColliderShape, EntityDesc, SceneFile};
    use olvin_transform::Parent;

    #[test]
    fn a_scene_terrain_spawns_chunks_with_a_material_and_its_collider() {
        let dir = std::env::temp_dir().join(format!("olvin_terrain_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A slope: height grows along x.
        let mut data = TerrainData::flat(65);
        for z in 0..65 {
            for x in 0..65 {
                data.set_height(x, z, x as f32 / 64.0);
            }
        }
        data.save(dir.join("terrains/hill.terrain")).unwrap();

        let mut res = Resources::default();
        res.insert(World::new());
        res.insert(Meshes::default());
        res.insert(RenderSettings::default());
        res.insert(RenderAssets::new(Some(dir.clone())));
        res.insert(olvin_render::Textures::default());
        res.insert(olvin_render::Materials::default());
        res.insert(olvin_render::Shaders::default());
        let mut e = EntityDesc::new(1, "Terrain");
        e.terrain = Some(TerrainDesc {
            data: "terrains/hill.terrain".into(),
            size: Vec3::new(64.0, 10.0, 64.0),
            layers: vec![TerrainLayer::default(), TerrainLayer::default()],
        });
        e.collider = Some(ColliderDesc::new(ColliderShape::Mesh));
        let scene = SceneFile {
            entities: vec![e],
            ..Default::default()
        };
        spawn(&res, &scene);

        let world = res.fetch::<World>();
        let chunks: Vec<_> = world
            .query::<(&MeshRenderer, &Parent)>()
            .iter()
            .map(|(m, _)| m.material)
            .collect();
        assert_eq!(chunks.len(), 4, "64 quads: 2 × 2 chunks");
        assert!(
            chunks.iter().all(|m| m.is_some()),
            "with the blended material"
        );
        let geometry = world
            .query::<&MeshGeometry>()
            .iter()
            .next()
            .cloned()
            .unwrap();
        assert_eq!(geometry.positions.len(), 65 * 65);
        let top = geometry
            .positions
            .iter()
            .map(|p| p.y)
            .fold(f32::MIN, f32::max);
        assert!(
            (top - 10.0).abs() < 0.01,
            "the file's heights, scaled by size.y"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
