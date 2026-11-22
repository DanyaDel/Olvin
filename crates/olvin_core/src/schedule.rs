use crate::Resources;
use crate::profile::{self, SystemScope};

/// Execution phases. `Startup` runs once (after the window exists, if any),
/// the rest run every frame in declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    Startup,
    First,
    PreUpdate,
    Update,
    PostUpdate,
    Render,
    Last,
}

impl Stage {
    pub const FRAME: [Stage; 6] = [
        Stage::First,
        Stage::PreUpdate,
        Stage::Update,
        Stage::PostUpdate,
        Stage::Render,
        Stage::Last,
    ];

    const COUNT: usize = 7;

    /// Scope name in the profiler.
    const SCOPE_NAMES: [&'static str; Self::COUNT] = [
        "stage Startup",
        "stage First",
        "stage PreUpdate",
        "stage Update",
        "stage PostUpdate",
        "stage Render",
        "stage Last",
    ];
}

/// A system runs once per stage invocation over the whole data set, never per object.
pub type System = Box<dyn FnMut(&mut Resources)>;

pub struct Schedule {
    stages: [Vec<(SystemScope, System)>; Stage::COUNT],
    stage_scopes: [SystemScope; Stage::COUNT],
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            stages: Default::default(),
            stage_scopes: std::array::from_fn(|i| SystemScope::register(Stage::SCOPE_NAMES[i])),
        }
    }
}

impl Schedule {
    pub fn add<F: FnMut(&mut Resources) + 'static>(&mut self, stage: Stage, system: F) {
        // The profiler shows a system under its function (or plugin) path.
        let scope = SystemScope::register(profile::system_name::<F>());
        self.stages[stage as usize].push((scope, Box::new(system)));
    }

    pub fn run(&mut self, stage: Stage, resources: &mut Resources) {
        let _stage = self.stage_scopes[stage as usize].enter();
        for (scope, system) in &mut self.stages[stage as usize] {
            let _system = scope.enter();
            system(resources);
        }
    }

    pub fn len(&self, stage: Stage) -> usize {
        self.stages[stage as usize].len()
    }
}
