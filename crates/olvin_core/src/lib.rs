//! Olvin core. Knows nothing about windows, GPUs or ECS storage: it only wires
//! plugins together through [`Resources`] and runs systems by [`Stage`].

mod app;
mod events;
pub mod logging;
mod paths;
mod plugin;
mod resources;
mod schedule;
mod surface;
mod time;

pub use app::{App, AppExit};
pub use events::{EventCursor, Events};
pub use logging::Logging;
pub use paths::{ASSETS_DIR_ENV, PROJECT_DIR_ENV, PathsSource, ProjectPaths};
pub use plugin::Plugin;
pub use resources::Resources;
pub use schedule::{Schedule, Stage, System};
pub use surface::{SurfaceTarget, WindowHandleSource};
pub use time::Time;
