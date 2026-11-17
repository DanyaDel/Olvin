//! ECS facade. Game code and other modules import ECS types from here, never
//! from `hecs` directly, so the storage backend can be swapped in one place.

pub use hecs::{
    Bundle, Component, CommandBuffer, Entity, EntityBuilder, NoSuchEntity, Or, Query,
    QueryBorrow, QueryMut, Satisfies, With, Without, World,
};

use olvin_core::{App, Plugin};

/// Inserts the [`World`] resource.
pub struct EcsPlugin;

impl Plugin for EcsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<World>();
    }
}
