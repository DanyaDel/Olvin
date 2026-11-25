//! Fault isolation: a crashed subsystem stops, the engine keeps running.
//!
//! Like a Linux box where PipeWire or the window manager died: the kernel
//! (here the loop, the ECS and the resources) goes on, the dead service logs
//! loudly and stays down until someone restarts it. A panic in a system stops
//! every system of the plugin that added it (the whole subsystem, not one
//! function), records a [`Fault`] in the [`Faults`] resource and writes a
//! large report to the log; [`Faults::restart`] brings the subsystem back.
//!
//! `OLVIN_FAULTS=abort` (CI, strict tests) turns isolation off: the panic
//! goes on and the process falls as before.

use std::any::Any;

use crate::Stage;

/// Environment variable selecting [`FaultPolicy`]: `abort` or `isolate`.
pub const FAULTS_ENV: &str = "OLVIN_FAULTS";

/// What a panic in a system does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FaultPolicy {
    /// The subsystem stops, the engine goes on (default).
    #[default]
    Isolate,
    /// The panic propagates (strict tests, CI).
    Abort,
}

impl FaultPolicy {
    /// From [`FAULTS_ENV`]; anything but `abort` isolates.
    pub fn from_env() -> Self {
        match std::env::var(FAULTS_ENV).as_deref() {
            Ok("abort") => Self::Abort,
            _ => Self::Isolate,
        }
    }
}

/// One crash of a subsystem.
#[derive(Clone, Debug)]
pub struct Fault {
    /// The plugin that added the system (`"core"` for the app's own systems).
    pub owner: &'static str,
    /// The system that panicked.
    pub system: &'static str,
    pub stage: Stage,
    /// The panic message.
    pub message: String,
    /// `Time::frame()` when it happened.
    pub frame: u64,
}

/// Crashed subsystems of the app, newest last. A resource every `App` has.
#[derive(Debug, Default)]
pub struct Faults {
    policy: Option<FaultPolicy>,
    list: Vec<Fault>,
    down: Vec<&'static str>,
    restart: Vec<String>,
}

impl Faults {
    pub fn policy(&self) -> FaultPolicy {
        self.policy.unwrap_or_else(FaultPolicy::from_env)
    }

    pub fn set_policy(&mut self, policy: FaultPolicy) {
        self.policy = Some(policy);
    }

    /// Every crash so far (also of subsystems restarted since).
    pub fn list(&self) -> &[Fault] {
        &self.list
    }

    /// Subsystems that are stopped now.
    pub fn down(&self) -> &[&'static str] {
        &self.down
    }

    pub fn is_down(&self, owner: &str) -> bool {
        self.down.contains(&owner)
    }

    /// Starts a stopped subsystem again at the next stage: its systems run as
    /// before (its `Startup` systems do not run twice). Whatever state the
    /// crash left in its resources stays; fix it first if it matters.
    pub fn restart(&mut self, owner: &str) {
        if self.is_down(owner) {
            self.restart.push(owner.to_owned());
        }
    }

    /// [`Self::restart`] for every stopped subsystem.
    pub fn restart_all(&mut self) {
        let down: Vec<String> = self.down.iter().map(|s| s.to_string()).collect();
        self.restart.extend(down);
    }

    pub(crate) fn record(&mut self, fault: Fault) {
        if !self.down.contains(&fault.owner) {
            self.down.push(fault.owner);
        }
        self.list.push(fault);
    }

    /// Restart requests, cleared; the schedule re-enables these owners.
    pub(crate) fn take_restarts(&mut self) -> Vec<String> {
        let wanted = std::mem::take(&mut self.restart);
        self.down.retain(|d| !wanted.iter().any(|w| w == d));
        wanted
    }
}

/// The text of a panic payload.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<not a string>".into())
}

/// The report written to the log when a subsystem goes down.
pub(crate) fn report(fault: &Fault, stopped: &[&'static str]) -> String {
    let mut out = String::new();
    out.push_str("================ SUBSYSTEM CRASHED ================\n");
    out.push_str(&format!("subsystem : {}\n", fault.owner));
    out.push_str(&format!("system    : {}\n", fault.system));
    out.push_str(&format!("stage     : {:?}\n", fault.stage));
    out.push_str(&format!("frame     : {}\n", fault.frame));
    out.push_str(&format!("panic     : {}\n", fault.message));
    out.push_str("stopped   :\n");
    for s in stopped {
        out.push_str(&format!("  - {s}\n"));
    }
    out.push_str(
        "The engine keeps running without this subsystem. The panic report with\n\
         a backtrace is in logs/crash-*.txt (RUST_BACKTRACE=1 for a full one).\n\
         Fix the cause and restart it: Faults::restart(\"<subsystem>\").\n",
    );
    out.push_str("===================================================");
    out
}
