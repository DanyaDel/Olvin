//! Olvin editor. The base (scene view, hierarchy, inspector, assets) lives
//! here; everything else comes from plugin crates passed to [`run`]. olvin-hub
//! generates the binary that lists the installed plugins.

mod app;
pub mod panels;
mod registry;
mod state;
pub mod theme;
mod workspace;

pub use registry::{EditorPlugin, Panel, PanelKind, Registry, TabAlloc, WorkspaceTemplate};
pub use state::{EditorState, LogLevel, LogLine, NewEntity};
pub use workspace::Tab;

// Re-exports so plugins build against exactly the editor's versions.
pub use eframe::egui;
pub use egui_dock;
pub use glam;
pub use olvin_scene;

/// Starts the editor. The first command-line argument is the project folder.
pub fn run(plugins: Vec<Box<dyn EditorPlugin>>) {
    let project = std::env::args_os().nth(1).map(std::path::PathBuf::from).map(|p| {
        p.canonicalize().unwrap_or(p)
    });

    let mut registry = Registry::default();
    panels::register(&mut registry);
    for plugin in &plugins {
        registry.plugins.push(plugin.name());
        plugin.build(&mut registry);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Olvin Editor")
            .with_app_id("olvin-editor")
            .with_inner_size([1600.0, 940.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "olvin-editor",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::EditorApp::new(registry, project)))
        }),
    );
    if let Err(err) = result {
        eprintln!("olvin-editor: {err}");
        std::process::exit(1);
    }
}
