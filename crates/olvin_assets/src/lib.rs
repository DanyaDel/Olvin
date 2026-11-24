//! Olvin asset pipeline, Unity-style: a project's `assets/` folder where every
//! file has a sibling `.meta` with a stable GUID and importer settings.
//! Pure data: no GPU, no window.
//!
//! - [`AssetRoot`]: paths, scanning, GUID index, move/delete keeping metas.
//! - [`load_image`], [`thumbnail`], [`ThumbnailCache`]: textures.
//! - [`import_gltf`], [`model_to_entities`], [`write_materials`]: models
//!   (meshes, materials, skins, animation clips, cameras).
//! - [`build_graph`], [`ModelData::animator_with`]: scene animation setups
//!   (clip names, `*.animgraph.ron`) as runtime graphs.
//! - [`AssetWatcher`]: debounced hot-reload events.
//! - [`AssetManager`]: counted [`Handle`]s, background loading, dependencies,
//!   unloading of unused assets, hot reload.

mod animation;
mod manager;
mod meta;
mod model;
mod root;
mod texture;
mod watcher;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

pub use animation::build_graph;
pub use manager::{
    AnyHandle, Asset, AssetChange, AssetManager, Dependency, Handle, Job, LoadState,
};
pub use meta::{
    Guid, Importer, META_EXT, Meta, ModelImport, TextureImport, is_ignored_name, meta_path,
};
pub use model::{
    CameraData, CameraProjection, MeshPrimitive, ModelData, ModelEntities, MorphClipData,
    MorphTargetData, MorphTrackData, NodeData, import_gltf, model_to_entities, write_materials,
};
pub use root::{AssetEntry, AssetKind, AssetRoot};
pub use texture::{
    ImageData, ImageFormat, ThumbnailCache, load_image, load_image_sized, thumbnail,
};
pub use watcher::{AssetEvent, AssetWatcher};

use olvin_core::{App, Plugin, Resources, Stage};

/// Inserts an [`AssetRoot`] resource for `root` (scanned once, creating
/// missing `.meta` files) and an [`AssetManager`] driven every frame.
/// Without a path it does nothing.
#[derive(Clone, Debug, Default)]
pub struct AssetsPlugin {
    pub root: Option<PathBuf>,
    /// Reload loaded assets when their files change (the manager's watcher).
    pub hot_reload: bool,
}

impl AssetsPlugin {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
            hot_reload: false,
        }
    }

    pub fn with_hot_reload(mut self) -> Self {
        self.hot_reload = true;
        self
    }
}

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut App) {
        if let Some(root) = &self.root {
            let mut assets = AssetRoot::new(root.clone());
            assets.scan();
            app.insert_resource(assets);
            let mut manager = AssetManager::new(root.clone());
            if self.hot_reload
                && let Err(err) = manager.enable_hot_reload()
            {
                log::warn!("asset hot reload off: {err}");
            }
            app.insert_resource(manager)
                .add_system(Stage::First, update_manager);
        }
    }
}

fn update_manager(res: &mut Resources) {
    res.fetch_mut::<AssetManager>().update();
}
