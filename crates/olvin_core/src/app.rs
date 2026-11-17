use std::any::TypeId;
use std::collections::HashSet;

use crate::{Plugin, Resources, Schedule, Stage, Time, time};

/// Set `requested` to stop the main loop after the current frame.
#[derive(Debug, Default)]
pub struct AppExit {
    pub requested: bool,
}

type Runner = Box<dyn FnOnce(App)>;

/// The engine instance: resources + schedule + the loop that drives them.
/// Which loop that is (winit, headless, a test harness) is decided by a plugin.
pub struct App {
    pub resources: Resources,
    schedule: Schedule,
    plugins: HashSet<TypeId>,
    runner: Option<Runner>,
    started: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            resources: Resources::default(),
            schedule: Schedule::default(),
            plugins: HashSet::new(),
            runner: None,
            started: false,
        };
        app.insert_resource(Time::default());
        app.insert_resource(AppExit::default());
        app.add_system(Stage::First, time::update_time);
        app
    }

    /// Builds a plugin. Adding the same plugin type twice is a no-op, so
    /// plugins can safely add the plugins they depend on.
    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        if self.plugins.insert(TypeId::of::<P>()) {
            log::debug!("plugin: {}", plugin.name());
            plugin.build(self);
        }
        self
    }

    pub fn has_plugin<P: Plugin>(&self) -> bool {
        self.plugins.contains(&TypeId::of::<P>())
    }

    pub fn add_system(
        &mut self,
        stage: Stage,
        system: impl FnMut(&mut Resources) + 'static,
    ) -> &mut Self {
        self.schedule.add(stage, system);
        self
    }

    pub fn insert_resource<T: 'static>(&mut self, value: T) -> &mut Self {
        self.resources.insert(value);
        self
    }

    /// Inserts `T::default()` unless the resource is already present.
    pub fn init_resource<T: Default + 'static>(&mut self) -> &mut Self {
        if !self.resources.contains::<T>() {
            self.resources.insert(T::default());
        }
        self
    }

    /// Replaces the main loop. The last plugin to call this wins.
    pub fn set_runner(&mut self, runner: impl FnOnce(App) + 'static) -> &mut Self {
        self.runner = Some(Box::new(runner));
        self
    }

    /// Hands control to the runner. Without one, runs a headless loop until exit.
    pub fn run(mut self) {
        let runner = self.runner.take().unwrap_or_else(|| Box::new(run_headless));
        runner(self);
    }

    /// Runs `Stage::Startup` once. Runners call it when the platform is ready.
    pub fn startup(&mut self) {
        if !self.started {
            self.started = true;
            self.schedule.run(Stage::Startup, &mut self.resources);
        }
    }

    /// Runs one frame.
    pub fn update(&mut self) {
        self.startup();
        for stage in Stage::FRAME {
            self.schedule.run(stage, &mut self.resources);
        }
    }

    pub fn exit_requested(&self) -> bool {
        self.resources.fetch::<AppExit>().requested
    }
}

fn run_headless(mut app: App) {
    while !app.exit_requested() {
        app.update();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Counter;
    impl Plugin for Counter {
        fn build(&self, app: &mut App) {
            app.init_resource::<u32>().add_system(Stage::Update, |res| {
                let mut n = res.fetch_mut::<u32>();
                *n += 1;
                if *n == 3 {
                    res.fetch_mut::<AppExit>().requested = true;
                }
            });
        }
    }

    #[test]
    fn headless_runs_until_exit_and_dedupes_plugins() {
        let mut app = App::new();
        app.add_plugin(Counter).add_plugin(Counter);
        assert!(app.has_plugin::<Counter>());

        let startups = Rc::new(Cell::new(0));
        let s = startups.clone();
        app.add_system(Stage::Startup, move |_| s.set(s.get() + 1));

        let result = Rc::new(Cell::new(0));
        let r = result.clone();
        app.set_runner(move |mut app| {
            while !app.exit_requested() {
                app.update();
            }
            r.set(*app.resources.fetch::<u32>());
        });
        app.run();

        // Plugin added twice still counts once per frame; startup ran once.
        assert_eq!(result.get(), 3);
        assert_eq!(startups.get(), 1);
    }
}
