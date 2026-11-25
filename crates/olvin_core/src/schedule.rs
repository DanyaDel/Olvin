use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use crate::faults::{self, Fault, FaultPolicy, Faults};
use crate::profile::{self, SystemScope};
use crate::{Resources, Time};

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

struct Entry {
    scope: SystemScope,
    name: &'static str,
    /// The plugin that added it; a crash stops all of its systems.
    owner: &'static str,
    system: System,
    stopped: bool,
}

pub struct Schedule {
    stages: [Vec<Entry>; Stage::COUNT],
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
        self.add_owned(stage, "core", system);
    }

    /// Adds a system that belongs to the subsystem `owner` (a plugin name).
    pub fn add_owned<F: FnMut(&mut Resources) + 'static>(
        &mut self,
        stage: Stage,
        owner: &'static str,
        system: F,
    ) {
        // The profiler shows a system under its function (or plugin) path.
        let name = profile::system_name::<F>();
        self.stages[stage as usize].push(Entry {
            scope: SystemScope::register(name),
            name,
            owner,
            system: Box::new(system),
            stopped: false,
        });
    }

    /// Runs the stage. A system that panics stops its whole subsystem (see
    /// [`crate::faults`]) unless the policy is [`FaultPolicy::Abort`].
    pub fn run(&mut self, stage: Stage, resources: &mut Resources) {
        let _stage = self.stage_scopes[stage as usize].enter();
        self.apply_restarts(resources);
        let policy = resources
            .get::<Faults>()
            .map_or_else(FaultPolicy::from_env, |f| f.policy());
        for i in 0..self.stages[stage as usize].len() {
            let entry = &mut self.stages[stage as usize][i];
            if entry.stopped {
                continue;
            }
            let result = {
                let _system = entry.scope.enter();
                let system = &mut entry.system;
                catch_unwind(AssertUnwindSafe(|| system(resources)))
            };
            if let Err(payload) = result {
                if policy == FaultPolicy::Abort {
                    resume_unwind(payload);
                }
                let fault = Fault {
                    owner: entry.owner,
                    system: entry.name,
                    stage,
                    message: faults::panic_message(payload.as_ref()),
                    frame: resources.get::<Time>().map_or(0, |t| t.frame()),
                };
                self.stop(fault, resources);
            }
        }
    }

    /// Stops every system of the fault's owner and reports it.
    fn stop(&mut self, fault: Fault, resources: &mut Resources) {
        let mut stopped = Vec::new();
        for entry in self.stages.iter_mut().flatten() {
            if entry.owner == fault.owner && !entry.stopped {
                entry.stopped = true;
                stopped.push(entry.name);
            }
        }
        log::error!("{}", faults::report(&fault, &stopped));
        if !resources.contains::<Faults>() {
            resources.insert(Faults::default());
        }
        resources.fetch_mut::<Faults>().record(fault);
    }

    fn apply_restarts(&mut self, resources: &mut Resources) {
        let Some(wanted) = resources.get_mut::<Faults>().map(|mut f| f.take_restarts()) else {
            return;
        };
        for owner in wanted {
            for entry in self.stages.iter_mut().flatten() {
                if entry.owner == owner {
                    entry.stopped = false;
                }
            }
            log::warn!("subsystem {owner} restarted");
        }
    }

    pub fn len(&self, stage: Stage) -> usize {
        self.stages[stage as usize].len()
    }
}
