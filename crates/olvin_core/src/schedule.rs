use crate::Resources;

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
}

/// A system runs once per stage invocation over the whole data set, never per object.
pub type System = Box<dyn FnMut(&mut Resources)>;

#[derive(Default)]
pub struct Schedule {
    stages: [Vec<System>; Stage::COUNT],
}

impl Schedule {
    pub fn add(&mut self, stage: Stage, system: impl FnMut(&mut Resources) + 'static) {
        self.stages[stage as usize].push(Box::new(system));
    }

    pub fn run(&mut self, stage: Stage, resources: &mut Resources) {
        for system in &mut self.stages[stage as usize] {
            system(resources);
        }
    }

    pub fn len(&self, stage: Stage) -> usize {
        self.stages[stage as usize].len()
    }
}
