//! Default hooks: faces selected in Edit Mode, and the material's base
//! texture behind the UVs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Color32, Pos2, Rect};
use olvin_editor::olvin_scene::material::MaterialDesc;

/// Faces of `id` whose vertices are all selected in Edit Mode.
pub fn edit_mode_faces(state: &EditorState, id: u64) -> Option<Vec<usize>> {
    let (entity, verts) = state.mesh_selection.as_ref()?;
    if *entity != id || verts.is_empty() {
        return None;
    }
    let poly = state.scene.get(id)?.poly.as_ref()?;
    Some(
        (0..poly.faces.len())
            .filter(|&f| poly.faces[f].iter().all(|v| verts.contains(v)))
            .collect(),
    )
}

#[derive(Default)]
struct Cache {
    thumbs: olvin_assets::ThumbnailCache,
    textures: HashMap<PathBuf, (Arc<olvin_assets::ImageData>, egui::TextureHandle)>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// Draws the base texture of the entity's material over the 0..1 square.
pub fn material_texture(
    painter: &egui::Painter,
    square: Rect,
    state: &EditorState,
    id: u64,
) -> bool {
    let Some(root) = state.assets_dir() else {
        return false;
    };
    let Some(rel) = state.scene.get(id).and_then(|e| e.material.clone()) else {
        return false;
    };
    let Some(tex) = MaterialDesc::load(root.join(rel))
        .ok()
        .and_then(|m| m.base_texture)
    else {
        return false;
    };
    let path = root.join(tex);
    let handle = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let image = c.thumbs.get(&path, 1024)?;
        if let Some((data, tex)) = c.textures.get(&path)
            && Arc::ptr_eq(data, &image)
        {
            return Some(tex.clone());
        }
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [image.width as usize, image.height as usize],
            &image.pixels,
        );
        let tex = painter.ctx().load_texture(
            format!("uv-bg:{}", path.display()),
            color,
            egui::TextureOptions::LINEAR,
        );
        c.textures.insert(path.clone(), (image, tex.clone()));
        Some(tex)
    });
    let Some(handle) = handle else { return false };
    // Texture row 0 is the top (V = 1 in Blender's convention used by the editor).
    painter.image(
        handle.id(),
        square,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    true
}
