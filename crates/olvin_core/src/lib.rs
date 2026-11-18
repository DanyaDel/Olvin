//! Olvin core. Knows nothing about windows, GPUs or ECS storage: it only wires
//! plugins together through [`Resources`] and runs systems by [`Stage`].

mod app;
pub mod logging;
mod plugin;
mod resources;
mod schedule;
mod surface;
mod time;

pub use app::{App, AppExit};
pub use logging::Logging;
pub use plugin::Plugin;
pub use resources::Resources;
pub use schedule::{Schedule, Stage, System};
pub use surface::{SurfaceTarget, WindowHandleSource};
pub use time::Time;
