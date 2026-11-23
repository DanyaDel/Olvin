//! Registration through the `App`, so a plugin's `build` reads in one chain.

use olvin_core::App;
use olvin_ecs::Component;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::message::{CommandCtx, MessageOptions, NetMessage, NetReceived, Verdict};
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

    /// Registers message type `T` under `name` and its `Events<NetReceived<T>>` queue.
    fn net_message<T: NetMessage>(&mut self, name: &str) -> &mut Self;

    /// [`NetAppExt::net_message`] with options (unreliable, size, rate, direction).
    fn net_message_with<T: NetMessage>(&mut self, name: &str, opts: MessageOptions) -> &mut Self;

    /// Registers a command: clients send it to the server, `validate` decides, and
    /// only accepted commands become `NetReceived<T>` events there. Rate limited
    /// (60 per second per client) by default.
    fn net_command<T: NetMessage>(
        &mut self,
        name: &str,
        validate: impl Fn(&CommandCtx, &T) -> Verdict + 'static,
    ) -> &mut Self;

    /// [`NetAppExt::net_command`] with explicit options.
    fn net_command_with<T: NetMessage>(
        &mut self,
        name: &str,
        opts: MessageOptions,
        validate: impl Fn(&CommandCtx, &T) -> Verdict + 'static,
    ) -> &mut Self;
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

    fn net_message<T: NetMessage>(&mut self, name: &str) -> &mut Self {
        self.net_message_with::<T>(name, MessageOptions::default())
    }

    fn net_message_with<T: NetMessage>(&mut self, name: &str, opts: MessageOptions) -> &mut Self {
        net(self).register_message::<T>(name, opts);
        self.add_event::<NetReceived<T>>()
    }

    fn net_command<T: NetMessage>(
        &mut self,
        name: &str,
        validate: impl Fn(&CommandCtx, &T) -> Verdict + 'static,
    ) -> &mut Self {
        self.net_command_with::<T>(name, MessageOptions::command(), validate)
    }

    fn net_command_with<T: NetMessage>(
        &mut self,
        name: &str,
        opts: MessageOptions,
        validate: impl Fn(&CommandCtx, &T) -> Verdict + 'static,
    ) -> &mut Self {
        net(self).register_command::<T>(name, opts, validate);
        self.add_event::<NetReceived<T>>()
    }
}
