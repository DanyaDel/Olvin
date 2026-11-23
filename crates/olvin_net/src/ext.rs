//! Registration through the `App`, so a plugin's `build` reads in one chain.

use olvin_core::App;
use olvin_ecs::Component;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::net::Net;
use crate::replicate::ReplicateOptions;

#[track_caller]
fn net(app: &App) -> std::cell::RefMut<'_, Net> {
    match app.resources.get_mut::<Net>() {
        Some(n) => n,
        None => panic!(
            "net: add NetPlugin to the App before registering components or messages \
             (app.add_plugin(NetPlugin::new(config)))"
        ),
    }
}

/// `app.replicate::<T>()`: the registration calls as `App` methods.
pub trait NetAppExt {
    /// See [`Net::replicate`].
    fn replicate<T>(&mut self) -> &mut Self
    where
        T: Component + Clone + Serialize + DeserializeOwned;

    /// See [`Net::replicate_with`].
    fn replicate_with<T>(&mut self, opts: ReplicateOptions<T>) -> &mut Self
    where
        T: Component + Clone + Serialize + DeserializeOwned;
}

impl NetAppExt for App {
    fn replicate<T>(&mut self) -> &mut Self
    where
        T: Component + Clone + Serialize + DeserializeOwned,
    {
        net(self).replicate::<T>();
        self
    }

    fn replicate_with<T>(&mut self, opts: ReplicateOptions<T>) -> &mut Self
    where
        T: Component + Clone + Serialize + DeserializeOwned,
    {
        net(self).replicate_with(opts);
        self
    }
}
