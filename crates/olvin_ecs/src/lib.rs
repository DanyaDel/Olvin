//! ECS facade. Game code and other modules import ECS types from here, never
//! from `hecs` directly, so the storage backend can be swapped in one place.

pub use hecs::{
    Bundle, CommandBuffer, Component, Entity, EntityBuilder, NoSuchEntity, Or, Query, QueryBorrow,
    QueryMut, Satisfies, With, Without, World,
};

use olvin_core::{App, Plugin};

/// Inserts the [`World`] resource.
pub struct EcsPlugin;

impl Plugin for EcsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<World>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Pos(f32);
    #[derive(Debug, PartialEq)]
    struct Vel(f32);
    struct Frozen;

    #[test]
    fn plugin_inserts_one_world() {
        let mut app = App::new();
        app.add_plugin(EcsPlugin);
        let e = app.resources.fetch_mut::<World>().spawn((Pos(1.0),));
        // Повторное добавление плагина не подменяет мир.
        app.add_plugin(EcsPlugin);
        assert!(app.resources.fetch::<World>().contains(e));
    }

    #[test]
    fn spawn_query_filter_despawn() {
        let mut world = World::new();
        let a = world.spawn((Pos(0.0), Vel(1.0)));
        let b = world.spawn((Pos(10.0), Vel(-2.0), Frozen));
        let c = world.spawn((Pos(5.0),));

        for (p, v) in world.query_mut::<(&mut Pos, &Vel)>().without::<&Frozen>() {
            p.0 += v.0;
        }
        assert_eq!(*world.get::<&Pos>(a).unwrap(), Pos(1.0));
        assert_eq!(*world.get::<&Pos>(b).unwrap(), Pos(10.0));
        assert_eq!(world.query::<&Pos>().with::<&Vel>().iter().count(), 2);
        assert_eq!(
            world
                .query::<&Pos>()
                .with::<Or<&Vel, &Frozen>>()
                .iter()
                .count(),
            2
        );

        world.despawn(c).unwrap();
        assert!(!world.contains(c));
        assert!(matches!(world.despawn(c), Err(NoSuchEntity)));
        assert_eq!(world.len(), 2);
    }

    #[test]
    fn components_and_command_buffer() {
        let mut world = World::new();
        let e = world.spawn((Pos(0.0),));
        world.insert_one(e, Vel(3.0)).unwrap();
        assert!(world.satisfies::<&Vel>(e));
        assert_eq!(world.remove_one::<Vel>(e).unwrap(), Vel(3.0));
        assert!(world.get::<&Vel>(e).is_err());

        // Отложенные правки: во время обхода мир менять нельзя, поэтому пишем в буфер.
        let mut cmd = CommandBuffer::new();
        for (entity, p) in world.query::<(Entity, &Pos)>().iter() {
            cmd.insert_one(entity, Vel(p.0 + 1.0));
            cmd.spawn((Pos(42.0),));
        }
        cmd.run_on(&mut world);
        assert_eq!(*world.get::<&Vel>(e).unwrap(), Vel(1.0));
        assert_eq!(world.len(), 2);

        let mut builder = EntityBuilder::new();
        builder.add(Pos(7.0)).add(Frozen);
        let built = world.spawn(builder.build());
        assert!(world.satisfies::<(&Pos, &Frozen)>(built));
    }
}
