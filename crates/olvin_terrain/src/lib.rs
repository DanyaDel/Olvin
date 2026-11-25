//! Heightmap terrains: the data and its file ([`TerrainData`]), chunked
//! meshes with levels of detail and collision ([`mesh`]), and how texture
//! layers are blended ([`look`]). Spawning from a scene lives in `olvin`
//! (feature `terrain`); the brushes live in the editor element.

mod data;
pub mod look;
pub mod mesh;

pub use data::{DEFAULT_RESOLUTION, MAX_RESOLUTION, MIN_RESOLUTION, TerrainData};
pub use olvin_scene::file::{TerrainDesc, TerrainLayer};

#[cfg(test)]
mod tests;
