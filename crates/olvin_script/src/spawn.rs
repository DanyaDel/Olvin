//! Prefab spawns requested by scripts (`ctx:spawn_prefab` in Lua).
//!
//! `olvin_script` cannot read scene files itself, so a script only queues a
//! [`SpawnRequest`]; the game (the `olvin` facade: `scene_file::run_prefab_spawns`)
//! drains the queue after the script pass, instantiates the prefab and tells the
//! requester with a `"spawned"` message (value: the new root's entity id).
//! Roots made this way carry [`ScriptSpawned`], which is what the alive cap counts.

use olvin_ecs::Entity;
use olvin_scene::Transform;

/// Requests one script pass may queue before `spawn_prefab` returns false.
pub const MAX_SPAWNS_PER_FRAME: usize = 16;
/// Prefab instances made by scripts that may exist at once (and be queued).
pub const MAX_SPAWNED_ALIVE: usize = 256;

/// Marks the root of a prefab instance a script spawned.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScriptSpawned;

/// One queued `spawn_prefab`.
#[derive(Clone, Debug)]
pub struct SpawnRequest {
    /// `*.prefab.ron`, relative to the project's `assets/`.
    pub asset: String,
    pub transform: Transform,
    /// The entity whose script asked; it gets the `"spawned"` message.
    pub requester: Entity,
}

/// Resource: spawns waiting for the game to carry them out.
#[derive(Default)]
pub struct SpawnRequests {
    pending: Vec<SpawnRequest>,
}

impl SpawnRequests {
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn push(&mut self, request: SpawnRequest) {
        self.pending.push(request);
    }

    /// Everything queued so far, emptying the queue.
    pub fn take(&mut self) -> Vec<SpawnRequest> {
        std::mem::take(&mut self.pending)
    }
}

/// A prefab path a script may ask for: relative to `assets/`, no `..`, no
/// absolute or backslash paths, ends with `.prefab.ron`.
pub fn check_prefab_path(path: &str) -> Result<(), String> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|part| part.is_empty() || part == "..")
        || !path.ends_with(".prefab.ron");
    if bad {
        return Err(format!(
            "prefab `{path}`: use a `*.prefab.ron` path relative to the assets folder, without `..`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefab_paths_stay_inside_assets() {
        assert!(check_prefab_path("prefabs/spawners/Crate.prefab.ron").is_ok());
        for bad in [
            "",
            "/etc/x.prefab.ron",
            "../x.prefab.ron",
            "prefabs/../../x.prefab.ron",
            "prefabs//x.prefab.ron",
            "C:/x.prefab.ron",
            "prefabs\\x.prefab.ron",
            "prefabs/x.scene.ron",
            "prefabs/x.lua",
        ] {
            assert!(check_prefab_path(bad).is_err(), "{bad}");
        }
    }
}
