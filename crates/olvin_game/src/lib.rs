//! Game runtime layer on top of the core: states (menu → game → pause) with
//! enter/exit hooks, background scene loading and save games. Spawning a loaded
//! scene into the world is the facade's job (`olvin::scene_file`); this crate only
//! delivers the parsed [`olvin_scene::file::SceneFile`], so it needs no GPU.

mod loader;
mod save;
mod save_sign;
mod state;

pub use loader::{LoadHandle, LoadStatus, SceneLoader, SceneLoaderPlugin};
#[cfg(feature = "physics")]
pub use save::BodyVelocity;
pub use save::{
    SAVE_VERSION, SaveData, SaveError, SaveId, SaveMeta, SavePlugin, SaveRegistry, SaveSlots,
    SavedEntity, SlotInfo,
};
pub use save_sign::{SavePolicy, SaveSigner, SaveStatus};
pub use state::{GameState, State, StateAppExt, StateChanged};
