//! The material preview in the inspector: a sphere wearing the material,
//! drawn by the engine's renderer (the same one as the scene view) into an
//! offscreen texture that egui shows as an image. It follows unsaved edits
//! at once, because the sphere's material is built from the description in
//! memory, not from the file.

use glam::camera::rh::{proj::directx, view};
use glam::{Mat4, Vec3};
use olvin::render::{Color, RenderSettings};
use olvin::render_assets::Registries;
use olvin::scene::material::MaterialDesc;

use crate::EditorState;
use crate::engine::settings_for;

/// Pixels of the preview texture (shown at half that in points, sharp on HiDPI).
pub const PREVIEW_PIXELS: u32 = 256;

/// Renders the preview of the material `rel` as `desc` describes it now.
/// `None` without a GPU (headless tests) or before the scene view's renderer exists.
pub(crate) fn render(
    state: &mut EditorState,
    rel: &str,
    desc: &MaterialDesc,
) -> Option<egui::TextureId> {
    let gpu = state.gpu.clone()?;
    let settings = preview_settings(state);
    let assets_dir = state.assets_dir();
    let preview = &mut state.engine_scene;
    preview.assets.set_root(assets_dir);
    let world = preview.material_preview_world(&format!("preview:{rel}"), desc);
    let mut renderer = gpu.renderer.write();
    let res = renderer
        .callback_resources
        .get_mut::<crate::render::Resources>()?;
    let target = res.engine.render_preview(
        &world,
        &mut preview.meshes,
        Registries {
            textures: &mut preview.textures,
            materials: &mut preview.materials,
            shaders: &mut preview.shaders,
        },
        view_proj(),
        &settings,
        PREVIEW_PIXELS,
    );
    if let Some(id) = target.egui_id {
        return Some(id);
    }
    let view = target.resolved.clone();
    let id = renderer.register_native_texture(&gpu.device, &view, eframe::wgpu::FilterMode::Linear);
    let res = renderer
        .callback_resources
        .get_mut::<crate::render::Resources>()?;
    res.engine.set_preview_egui_id(id);
    Some(id)
}

/// The sphere seen slightly from above, filling most of the square.
fn view_proj() -> Mat4 {
    let eye = Vec3::new(0.0, 0.35, 2.4);
    directx::perspective(30f32.to_radians(), 1.0, 0.05, 20.0)
        * view::look_at_mat4(eye, Vec3::ZERO, Vec3::Y)
}

/// The game's look (exposure, tonemapping, ambient, sky) with a studio key
/// light instead of the scene's lights, no fog and no shadows.
fn preview_settings(state: &mut EditorState) -> RenderSettings {
    let mut s = settings_for(&state.scene);
    s.scene_lights = false;
    s.light_dir = Vec3::new(-0.55, -0.6, -0.6).normalize();
    s.light_color = Color([1.1, 1.06, 1.0, 1.0]);
    s.light_shadows = false;
    s.fog = None;
    s.height_fog = None;
    s.sky = state.engine_scene.sky(&state.scene.environment.skybox);
    s
}
