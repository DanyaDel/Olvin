//! olvin-hub core: the module catalog, game projects and hub settings.
//! The CLI and GUI are thin front-ends over this crate.

mod build;
mod catalog;
mod config;
mod editor;
mod library;
mod project;
mod template;

pub use build::{ArchiveFormat, BuildOptions, BuildReport};
pub use catalog::{Catalog, EditorModule, Module, Source};
pub use config::{ENGINE_GIT, EngineSource, GuiSettings, HubConfig};
pub use editor::{EditorInstall, engine_fingerprint};
pub use library::{
    LibraryRow, library_add, library_describe, library_for, library_installed, library_rows,
};
pub use olvin_library::{Category, ImportReport, Library, OnConflict, Status};
pub use project::{Project, name_problem};
