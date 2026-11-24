//! olvin-hub core: the module catalog, game projects and hub settings.
//! The CLI and GUI are thin front-ends over this crate.

mod build;
mod catalog;
mod config;
mod editor;
mod library;
mod project;
mod signing;
mod template;

pub use build::{ArchiveFormat, BuildOptions, BuildReport};
pub use catalog::{Catalog, EditorModule, Module, Source};
pub use config::{ENGINE_GIT, EngineSource, GuiSettings, HubConfig};
pub use editor::{EditorInstall, engine_fingerprint};
pub use library::{
    LibraryRow, library_add, library_describe, library_for, library_installed, library_rows,
};
pub use olvin_core::integrity::{IntegrityReport, PublicKey, SignatureState, verify_dir};
pub use olvin_library::{Category, ImportReport, Library, OnConflict, Status};
pub use olvin_trust::{
    EXPLANATION as TRUST_EXPLANATION, MODE_ENV as TRUST_MODE_ENV, TRUST_ENV, TrustStore,
    env_read_only, env_trusts, warning as trust_warning,
};
pub use project::{Project, name_problem};
pub use signing::{
    KEY_DATA_ENV, KEY_ENV, Signing, default_key_path, generate_key, public_key_path,
    read_public_key,
};
