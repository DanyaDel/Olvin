//! Olvin asset pipeline, Unity-style: a project's `assets/` folder where every
//! file has a sibling `.meta` with a stable GUID and importer settings.
//! Pure data: no GPU, no window.
//!
//! - [`AssetRoot`]: paths, scanning, GUID index, move/delete keeping metas.
//! - [`load_image`], [`thumbnail`], [`ThumbnailCache`]: textures.
//! - [`import_gltf`], [`model_to_entities`], [`write_materials`]: models.
//! - [`AssetWatcher`]: debounced hot-reload events.

mod meta;
mod model;
mod root;
mod texture;
mod watcher;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

pub use meta::{
    Guid, Importer, META_EXT, Meta, ModelImport, TextureImport, is_ignored_name, meta_path,
};
pub use model::{
    MeshPrimitive, ModelData, ModelEntities, NodeData, import_gltf, model_to_entities,
    write_materials,
};
pub use root::{AssetEntry, AssetKind, AssetRoot};
pub use texture::{
    ImageData, ImageFormat, ThumbnailCache, load_image, load_image_sized, thumbnail,
};
pub use watcher::{AssetEvent, AssetWatcher};

use olvin_core::{App, Plugin};

/// Inserts an [`AssetRoot`] resource for `root` (scanned once, creating
/// missing `.meta` files). Without a path it does nothing.
#[derive(Clone, Debug, Default)]
pub struct AssetsPlugin {
    pub root: Option<PathBuf>,
}

impl AssetsPlugin {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
        }
    }
}

impl Plugin for AssetsPlugin {
    fn build(&self, app: &mut App) {
        if let Some(root) = &self.root {
            let mut assets = AssetRoot::new(root.clone());
            assets.scan();
            app.insert_resource(assets);
        }
    }
}
