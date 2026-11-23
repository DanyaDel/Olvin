//! Olvin editor. The base (scene view, hierarchy, inspector, assets) lives
//! here; everything else comes from plugin crates passed to [`run`]. olvin-hub
//! generates the binary that lists the installed plugins.

mod app;
pub mod engine;
pub mod ext;
pub mod materials;
pub mod panels;
pub mod prefabs;
pub mod prefs;
mod registry;
mod render;
pub mod settings;
mod state;
pub mod theme;
pub mod widgets;
mod workspace;

pub use app::{EditorApp, TopBarRects};
pub use registry::{
    EditorPlugin, FrameHook, Panel, PanelKind, Registry, TabAlloc, WorkspaceTemplate,
};
pub use render::{DEPTH_FORMAT, MSAA};
pub use state::{EditorState, LogLevel, LogLine, NewEntity, PrefabReport, SnapSettings, tools};
pub use workspace::Tab;

// Re-exports so plugins build against exactly the editor's versions.
pub use eframe::egui;
pub use egui_dock;
pub use glam;
pub use olvin;
pub use olvin_scene;

/// Window and taskbar icon: `assets/logo/png/olvin-icon-256.png` (mark on the dark tile).
fn window_icon() -> egui::IconData {
    const PNG: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/logo/png/olvin-icon-256.png"
    ));
    // invariant: PNG is embedded at build time from a checked-in file.
    eframe::icon_data::from_png_bytes(PNG).expect("olvin-icon-256.png is not a valid PNG")
}

/// The base panels plus everything `plugins` add.
pub fn registry(plugins: &[Box<dyn EditorPlugin>]) -> Registry {
    let mut registry = Registry::default();
    panels::register(&mut registry);
    for plugin in plugins {
        registry.plugins.push(plugin.name());
        plugin.build(&mut registry);
    }
    registry
}

/// The editor app inside eframe's creation context: theme, GPU scene view
/// resources, panels. The context needs the wgpu renderer with
/// [`DEPTH_FORMAT`] depth and [`MSAA`] samples, like [`run`] sets up.
pub fn build_app(
    cc: &eframe::CreationContext<'_>,
    registry: Registry,
    project: Option<std::path::PathBuf>,
) -> Result<EditorApp, &'static str> {
    theme::apply(&cc.egui_ctx);
    let gpu = cc
        .wgpu_render_state
        .as_ref()
        .ok_or("olvin-editor needs the wgpu renderer")?;
    render::init(gpu);
    Ok(EditorApp::new(registry, project, Some(gpu.clone())))
}

/// Starts the editor. The first command-line argument is the project folder.
pub fn run(plugins: Vec<Box<dyn EditorPlugin>>) {
    let project = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .map(|p| p.canonicalize().unwrap_or(p));

    // Log and crash reports go next to the project, like the game's own.
    let mut logging = olvin::core::Logging::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn");
    if let Some(project) = &project {
        logging = logging.file(project.join("logs/editor.log"));
    }
    logging.init();

    let registry = registry(&plugins);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Olvin Editor")
            .with_app_id("olvin-editor")
            .with_inner_size([1600.0, 940.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(window_icon()),
        // The scene view draws into egui's pass and needs depth + MSAA.
        depth_buffer: render::DEPTH_BITS,
        multisampling: render::MSAA as u16,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let result = eframe::run_native(
        "olvin-editor",
        options,
        Box::new(move |cc| Ok(Box::new(build_app(cc, registry, project)?))),
    );
    if let Err(err) = result {
        eprintln!("olvin-editor: {err}");
        std::process::exit(1);
    }
}
