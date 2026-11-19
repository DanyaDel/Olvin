//! olvin-hub core: the module catalog, game projects and hub settings.
//! The CLI and GUI are thin front-ends over this crate.

mod build;
mod catalog;
mod config;
mod editor;
mod project;
mod template;

pub use build::{ArchiveFormat, BuildOptions, BuildReport};
pub use catalog::{Catalog, EditorModule, Module, Source};
pub use config::{ENGINE_GIT, EngineSource, GuiSettings, HubConfig};
pub use editor::{EditorInstall, engine_fingerprint};
pub use project::Project;
