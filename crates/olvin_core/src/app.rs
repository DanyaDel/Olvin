use std::any::TypeId;
use std::collections::HashSet;

use crate::{Events, Faults, Plugin, ProjectPaths, Resources, Schedule, Stage, Time, events, time};

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
    /// Plugins being built, innermost last: their systems belong to the last one.
    building: Vec<&'static str>,
    events: HashSet<TypeId>,
    runner: Option<Runner>,
    started: bool,
    /// `update` closes the profiler frame itself (off when a host does).
    profile_frames: bool,
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
            building: Vec::new(),
            events: HashSet::new(),
            runner: None,
            started: false,
            profile_frames: true,
        };
        crate::profile::init_from_env();
        app.insert_resource(Time::default());
        app.insert_resource(AppExit::default());
        app.insert_resource(ProjectPaths::detect().clone());
        app.insert_resource(Faults::default());
        app.add_system(Stage::First, time::update_time);
        app
    }

    /// Builds a plugin. Adding the same plugin type twice is a no-op, so
    /// plugins can safely add the plugins they depend on.
    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        if self.plugins.insert(TypeId::of::<P>()) {
            log::debug!("plugin: {}", plugin.name());
            self.building.push(short_name(plugin.name()));
            plugin.build(self);
            self.building.pop();
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
        let owner = self.building.last().copied().unwrap_or("core");
        self.schedule.add_owned(stage, owner, system);
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

    /// Registers the event queue `Events<T>` and swaps its buffers at the start
    /// of every frame (`Stage::First`). Registering the same type twice is a no-op.
    pub fn add_event<T: 'static>(&mut self) -> &mut Self {
        if self.events.insert(TypeId::of::<T>()) {
            self.init_resource::<Events<T>>();
            self.add_system(Stage::First, events::update_events::<T>);
        }
        self
    }

    /// Replaces the main loop. The last plugin to call this wins.
    pub fn set_runner(&mut self, runner: impl FnOnce(App) + 'static) -> &mut Self {
        self.runner = Some(Box::new(runner));
        self
    }

    /// Hands control to the runner. Without one, runs a headless loop until exit.
    /// An exit already requested while the app was being built (a failed
    /// integrity check, say) means it never starts: no window, no runner.
    pub fn run(mut self) {
        if self.exit_requested() {
            log::info!("exit requested before start, not running");
            return;
        }
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

    /// Whether [`Self::update`] marks a profiler frame boundary (default). A
    /// host that drives several updates per frame, or its own, turns it off
    /// and calls [`crate::profile::new_frame`] itself.
    pub fn set_profile_frames(&mut self, on: bool) -> &mut Self {
        self.profile_frames = on;
        self
    }

    /// Runs one frame.
    pub fn update(&mut self) {
        if self.profile_frames {
            crate::profile::new_frame();
        }
        self.startup();
        for stage in Stage::FRAME {
            self.schedule.run(stage, &mut self.resources);
        }
    }

    pub fn exit_requested(&self) -> bool {
        self.resources.fetch::<AppExit>().requested
    }
}

/// `olvin_physics::PhysicsPlugin` → `PhysicsPlugin`, generics included.
fn short_name(name: &'static str) -> &'static str {
    let head = name.split('<').next().unwrap_or(name);
    match head.rfind("::") {
        Some(i) => &name[i + 2..],
        None => name,
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
    use crate::FaultPolicy;
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
    fn an_exit_requested_while_building_stops_run_before_the_runner() {
        let mut app = App::new();
        app.resources.fetch_mut::<AppExit>().requested = true;
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        app.set_runner(move |_| r.set(true));
        app.run();
        assert!(!ran.get());
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

    /// Panics on its third frame; its other system counts frames.
    struct Crashy;
    struct CrashyTicks(u32);
    struct Alive(u32);
    impl Plugin for Crashy {
        fn build(&self, app: &mut App) {
            app.insert_resource(CrashyTicks(0))
                .add_system(Stage::Update, |res| {
                    let frame = res.fetch::<crate::Time>().frame();
                    if frame == 2 && !res.contains::<bool>() {
                        panic!("crashy broke on purpose");
                    }
                })
                .add_system(Stage::PostUpdate, |res| {
                    res.fetch_mut::<CrashyTicks>().0 += 1;
                });
        }
    }

    fn crashy_app(policy: FaultPolicy) -> App {
        let mut app = App::new();
        app.resources.fetch_mut::<Faults>().set_policy(policy);
        app.insert_resource(Alive(0));
        app.add_plugin(Crashy);
        app.add_system(Stage::Update, |res| res.fetch_mut::<Alive>().0 += 1);
        app
    }

    #[test]
    fn a_crashed_plugin_stops_and_the_rest_keeps_running() {
        let mut app = crashy_app(FaultPolicy::Isolate);
        for _ in 0..5 {
            app.update();
        }
        assert_eq!(app.resources.fetch::<Alive>().0, 5, "core systems go on");
        assert_eq!(
            app.resources.fetch::<CrashyTicks>().0,
            2,
            "the whole subsystem stopped, not only the system that panicked"
        );
        {
            let faults = app.resources.fetch::<Faults>();
            assert_eq!(faults.down(), ["Crashy"]);
            let fault = &faults.list()[0];
            assert_eq!(fault.owner, "Crashy");
            assert_eq!(fault.stage, Stage::Update);
            assert_eq!(fault.message, "crashy broke on purpose");
            assert_eq!(fault.frame, 2);
        }
        // "Fixed": restart brings it back.
        app.insert_resource(true);
        app.resources.fetch_mut::<Faults>().restart("Crashy");
        app.update();
        assert!(app.resources.fetch::<Faults>().down().is_empty());
        assert_eq!(app.resources.fetch::<CrashyTicks>().0, 3);
    }

    #[test]
    fn abort_policy_lets_the_panic_through() {
        let mut app = crashy_app(FaultPolicy::Abort);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for _ in 0..5 {
                app.update();
            }
        }));
        assert!(result.is_err());
    }

    #[test]
    fn plugin_names_are_short() {
        assert_eq!(short_name("olvin_physics::PhysicsPlugin"), "PhysicsPlugin");
        assert_eq!(short_name("a::Gen<b::C>"), "Gen<b::C>");
        assert_eq!(short_name("Plain"), "Plain");
    }
}
