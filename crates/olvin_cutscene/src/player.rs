//! Playing cutscenes: [`Cutscenes`] holds the running ones, [`CutscenePlugin`]
//! advances them every frame.

use std::collections::HashMap;
use std::sync::Arc;

use olvin_core::{App, EventCursor, Events, Plugin, ProjectPaths, Resources, Stage, Time};
use olvin_ecs::{Entity, World};
use olvin_scene::{ActiveCamera, Camera, Name, Transform};
use olvin_script::{ScriptMessage, ScriptValue};

use crate::data::{CueAction, CueValue, Cutscene, safe_path};

/// Handle of a running cutscene.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CutsceneId(u64);

/// Event: a cutscene ended (reached its end, was skipped or was stopped).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutsceneFinished {
    pub id: CutsceneId,
    pub name: String,
    /// Ended by [`Cutscenes::skip`] or [`Cutscenes::stop`], not by running out.
    pub cut_short: bool,
}

/// What a run was asked to do besides playing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ask {
    None,
    Skip,
    Stop,
}

struct Run {
    id: CutsceneId,
    cutscene: Arc<Cutscene>,
    time: f32,
    speed: f32,
    looping: bool,
    paused: bool,
    next_cue: usize,
    /// The entity of each track, found when the run starts (and again if it died).
    targets: Vec<Option<Entity>>,
    set_camera: bool,
    ask: Ask,
}

/// Resource: the running cutscenes and the loaded ones.
#[derive(Default)]
pub struct Cutscenes {
    runs: Vec<Run>,
    next_id: u64,
    cache: HashMap<String, Arc<Cutscene>>,
    messages: EventCursor,
}

/// Most cutscenes that may run at once.
const MAX_RUNNING: usize = 16;

impl Cutscenes {
    /// Starts `cutscene` from its beginning. `None` when too many are running.
    pub fn play(&mut self, cutscene: Arc<Cutscene>) -> Option<CutsceneId> {
        if self.runs.len() >= MAX_RUNNING {
            log::warn!(
                "cutscene `{}` not started: {MAX_RUNNING} are already running",
                cutscene.name
            );
            return None;
        }
        let id = CutsceneId(self.next_id);
        self.next_id += 1;
        let tracks = cutscene.tracks.len();
        self.runs.push(Run {
            id,
            cutscene,
            time: 0.0,
            speed: 1.0,
            looping: false,
            paused: false,
            next_cue: 0,
            targets: vec![None; tracks],
            set_camera: false,
            ask: Ask::None,
        });
        Some(id)
    }

    /// Plays in a loop until stopped.
    pub fn play_looping(&mut self, cutscene: Arc<Cutscene>) -> Option<CutsceneId> {
        let id = self.play(cutscene)?;
        self.run_mut(id)?.looping = true;
        Some(id)
    }

    /// Loads `path` (relative to `assets`, cached) and plays it.
    pub fn play_file(&mut self, assets: &std::path::Path, path: &str) -> Option<CutsceneId> {
        if !safe_path(path) {
            log::warn!("cutscene path `{path}` must stay inside the assets folder");
            return None;
        }
        let cutscene = match self.cache.get(path) {
            Some(c) => c.clone(),
            None => match Cutscene::load(assets.join(path)) {
                Ok(c) => {
                    let c = Arc::new(c);
                    self.cache.insert(path.to_owned(), c.clone());
                    c
                }
                Err(e) => {
                    log::warn!("cutscene `{path}` not played: {e}");
                    return None;
                }
            },
        };
        self.play(cutscene)
    }

    fn run_mut(&mut self, id: CutsceneId) -> Option<&mut Run> {
        self.runs.iter_mut().find(|r| r.id == id)
    }

    /// Ends it now, leaving the objects where they are. False if not running.
    pub fn stop(&mut self, id: CutsceneId) -> bool {
        self.ask(id, Ask::Stop)
    }

    /// Jumps to the end: the objects take their final places and the message
    /// and camera cues still ahead happen (sounds do not). False if not running.
    pub fn skip(&mut self, id: CutsceneId) -> bool {
        self.ask(id, Ask::Skip)
    }

    fn ask(&mut self, id: CutsceneId, ask: Ask) -> bool {
        match self.run_mut(id) {
            Some(r) => {
                r.ask = ask;
                true
            }
            None => false,
        }
    }

    pub fn pause(&mut self, id: CutsceneId, paused: bool) -> bool {
        match self.run_mut(id) {
            Some(r) => {
                r.paused = paused;
                true
            }
            None => false,
        }
    }

    pub fn set_speed(&mut self, id: CutsceneId, speed: f32) -> bool {
        match self.run_mut(id) {
            Some(r) if speed.is_finite() => {
                r.speed = speed.max(0.0);
                true
            }
            _ => false,
        }
    }

    pub fn is_playing(&self, id: CutsceneId) -> bool {
        self.runs.iter().any(|r| r.id == id)
    }

    /// Whether any cutscene is running (a game can hold the player still).
    pub fn is_busy(&self) -> bool {
        !self.runs.is_empty()
    }

    /// Seconds into the cutscene.
    pub fn time(&self, id: CutsceneId) -> Option<f32> {
        self.runs.iter().find(|r| r.id == id).map(|r| r.time)
    }

    /// Stops or skips every running cutscene.
    pub fn skip_all(&mut self) {
        self.runs.iter_mut().for_each(|r| r.ask = Ask::Skip);
    }

    pub fn stop_all(&mut self) {
        self.runs.iter_mut().for_each(|r| r.ask = Ask::Stop);
    }
}

fn find(world: &World, name: &str) -> Option<Entity> {
    world
        .query::<(Entity, &Name)>()
        .iter()
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
}

fn value(v: &CueValue) -> ScriptValue {
    match v {
        CueValue::Nil => ScriptValue::Nil,
        CueValue::Bool(b) => ScriptValue::Bool(*b),
        CueValue::Number(n) => ScriptValue::Number(*n),
        CueValue::Text(t) => ScriptValue::Str(t.clone()),
    }
}

/// Draws `entity`'s camera (or the game's own with `None`).
fn set_active_camera(world: &mut World, entity: Option<Entity>) {
    let old: Vec<Entity> = world
        .query::<(Entity, &ActiveCamera)>()
        .iter()
        .map(|(e, _)| e)
        .collect();
    for e in old {
        let _ = world.remove_one::<ActiveCamera>(e);
    }
    if let Some(e) = entity.filter(|e| world.get::<&Camera>(*e).is_ok()) {
        let _ = world.insert_one(e, ActiveCamera);
    }
}

/// Everything a cue may touch besides the world.
struct Out<'a> {
    messages: Option<&'a mut Events<ScriptMessage>>,
    #[cfg(feature = "audio")]
    audio: Option<&'a mut olvin_audio::Audio>,
}

fn fire(run: &mut Run, index: usize, world: &mut World, out: &mut Out<'_>, skipping: bool) {
    let cutscene = run.cutscene.clone();
    match &cutscene.cues[index].action {
        CueAction::Camera(name) => {
            if name.is_empty() {
                set_active_camera(world, None);
                run.set_camera = false;
            } else if let Some(e) = find(world, name) {
                set_active_camera(world, Some(e));
                run.set_camera = true;
            } else {
                log::warn!("cutscene `{}`: no camera object `{name}`", cutscene.name);
            }
        }
        CueAction::Message {
            name,
            value: v,
            target,
        } => {
            let target = match target {
                Some(t) => match find(world, t) {
                    Some(e) => Some(e),
                    None => {
                        log::warn!(
                            "cutscene `{}`: no object `{t}` for message `{name}`",
                            cutscene.name
                        );
                        return;
                    }
                },
                None => None,
            };
            if let Some(events) = out.messages.as_deref_mut() {
                events.send(ScriptMessage {
                    name: name.clone(),
                    value: value(v),
                    from: None,
                    target,
                });
            }
        }
        CueAction::Sound { path, volume } => {
            #[cfg(feature = "audio")]
            if !skipping && let Some(audio) = out.audio.as_deref_mut() {
                audio.play_with(
                    path,
                    olvin_audio::PlayParams {
                        volume: *volume,
                        ..Default::default()
                    },
                );
            }
            #[cfg(not(feature = "audio"))]
            let _ = (path, volume, skipping);
        }
    }
}

/// Moves every track's object to where the tracks put it at the run's time.
fn apply_tracks(run: &mut Run, world: &mut World) {
    let cutscene = run.cutscene.clone();
    for (i, track) in cutscene.tracks.iter().enumerate() {
        let alive = run.targets[i].filter(|e| world.contains(*e));
        let target = alive.or_else(|| find(world, &track.target));
        run.targets[i] = target;
        let Some(e) = target else {
            continue;
        };
        if let Ok(mut t) = world.get::<&mut Transform>(e) {
            *t = track.sample(run.time, *t);
        }
    }
}

fn advance(run: &mut Run, dt: f32, world: &mut World, out: &mut Out<'_>) -> Option<bool> {
    let duration = run.cutscene.duration;
    let skipping = run.ask == Ask::Skip;
    if run.ask == Ask::Stop {
        return Some(true);
    }
    if run.paused && !skipping {
        return None;
    }
    let mut ended = skipping;
    if skipping {
        run.time = duration;
    } else {
        run.time += dt * run.speed;
        if run.time >= duration {
            ended = true;
        }
    }
    let now = if ended { duration } else { run.time };
    run.time = now;
    let cues = run.cutscene.cues.len();
    while run.next_cue < cues && (ended || run.cutscene.cues[run.next_cue].time <= now) {
        let i = run.next_cue;
        run.next_cue += 1;
        fire(run, i, world, out, skipping);
    }
    apply_tracks(run, world);
    if !ended {
        return None;
    }
    if run.looping && !skipping && duration > 0.0 {
        run.time = 0.0;
        run.next_cue = 0;
        return None;
    }
    Some(skipping)
}

/// Reads the `cutscene:*` messages scripts send.
fn handle_messages(res: &Resources, cutscenes: &mut Cutscenes) {
    let Some(events) = res.get::<Events<ScriptMessage>>() else {
        return;
    };
    let assets = res.get::<ProjectPaths>().map(|p| p.assets.clone());
    let wanted: Vec<ScriptMessage> = events
        .read(&mut cutscenes.messages)
        .filter(|m| m.name.starts_with("cutscene:"))
        .cloned()
        .collect();
    drop(events);
    for m in wanted {
        match (m.name.as_str(), &m.value) {
            ("cutscene:play", ScriptValue::Str(path)) => {
                if let Some(assets) = &assets {
                    cutscenes.play_file(assets, path);
                }
            }
            ("cutscene:skip", _) => cutscenes.skip_all(),
            ("cutscene:stop", _) => cutscenes.stop_all(),
            (other, _) => log::warn!("unknown cutscene message `{other}` (or a bad value)"),
        }
    }
}

fn run_cutscenes(res: &mut Resources) {
    let dt = res.get::<Time>().map_or(0.0, |t| t.delta());
    let mut cutscenes = res.fetch_mut::<Cutscenes>();
    handle_messages(res, &mut cutscenes);
    if cutscenes.runs.is_empty() {
        return;
    }
    let mut world = res.fetch_mut::<World>();
    let mut messages = res.get_mut::<Events<ScriptMessage>>();
    #[cfg(feature = "audio")]
    let mut audio = res.get_mut::<olvin_audio::Audio>();
    let mut out = Out {
        messages: messages.as_deref_mut(),
        #[cfg(feature = "audio")]
        audio: audio.as_deref_mut(),
    };
    let mut finished = Vec::new();
    let mut runs = std::mem::take(&mut cutscenes.runs);
    runs.retain_mut(|run| match advance(run, dt, &mut world, &mut out) {
        None => true,
        Some(cut_short) => {
            if run.set_camera {
                set_active_camera(&mut world, None);
            }
            finished.push(CutsceneFinished {
                id: run.id,
                name: run.cutscene.name.clone(),
                cut_short,
            });
            false
        }
    });
    cutscenes.runs = runs;
    drop(cutscenes);
    for f in &finished {
        if let Some(events) = out.messages.as_deref_mut() {
            events.send(ScriptMessage::new(
                "cutscene_finished",
                ScriptValue::Str(f.name.clone()),
            ));
        }
    }
    drop(messages);
    if let Some(mut events) = res.get_mut::<Events<CutsceneFinished>>() {
        events.send_batch(finished);
    }
}

/// Adds [`Cutscenes`] and advances the running ones in `Stage::Update`.
pub struct CutscenePlugin;

impl Plugin for CutscenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<World>();
        app.init_resource::<Cutscenes>();
        app.add_event::<ScriptMessage>();
        app.add_event::<CutsceneFinished>();
        app.add_system(Stage::Update, run_cutscenes);
    }
}

#[cfg(test)]
mod tests;
