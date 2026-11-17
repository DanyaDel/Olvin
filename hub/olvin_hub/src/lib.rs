//! olvin-hub core: the module catalog, game projects and hub settings.
//! The CLI and GUI are thin front-ends over this crate.

mod catalog;
mod config;
mod project;
mod template;

pub use catalog::{Catalog, Module, Source};
pub use config::{ENGINE_GIT, EngineSource, HubConfig};
pub use project::Project;
