//! The character packs, imported into a temp project and played headless:
//! `HeadlessPlugins` + the Lua plugin + the pack's demo scene, fixed 1/60 s
//! frames, keys and mouse fed into the `Input` resource like a window would.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use glam::EulerRot;
use olvin::input::Input;
use olvin::prelude::*;
use olvin::script::lua::{LuaLimits, LuaPlugin};
use olvin::window::KeyCode;
use olvin_library::{ImportOptions, Library};

const DT: f32 = 1.0 / 60.0;

/// Warnings and errors from every test in this binary (Lua errors go to the log).
static PROBLEMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            PROBLEMS
                .lock()
                .unwrap()
                .push(format!("[{}] {}: {}", r.level(), r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

fn capture_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = log::set_logger(&Capture);
        log::set_max_level(log::LevelFilter::Warn);
    });
}

/// No warning or error was logged so far (by any test of this binary).
fn assert_clean_log() {
    let problems = PROBLEMS.lock().unwrap();
    assert!(
        problems.is_empty(),
        "logged problems:\n{}",
        problems.join("\n")
    );
}

struct Rig {
    app: App,
    project: PathBuf,
    _dir: tempfile::TempDir,
}

/// Imports `pack` into a fresh project (`edit` may rewrite the imported files
/// first, to change script parameters) and loads its demo scene.
fn rig(pack: &str, edit: impl FnOnce(&Path)) -> Rig {
    capture_log();
    let dir = tempfile::tempdir().unwrap();
    let library = Library::embedded();
    let report = library
        .import(pack, dir.path(), &ImportOptions::default())
        .unwrap();
    assert!(report.completed, "{report:?}");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    edit(&dir.path().join("assets"));

    let mut app = App::new();
    app.insert_resource(ProjectPaths::new(dir.path()));
    app.insert_resource(Time::with_fixed_delta(DT));
    app.add_plugin(HeadlessPlugins::default());
    let scripts = dir.path().join("assets/scripts");
    app.add_plugin(LuaPlugin {
        dir: scripts,
        hot_reload: false,
        limits: LuaLimits::default(),
    });
    let scene = dir
        .path()
        .join(format!("assets/scenes/{pack}/demo.scene.ron"));
    let spawned = olvin::scene_file::load(&app.resources, &scene).unwrap();
    assert!(spawned > 10, "the demo scene is not empty");
    let mut rig = Rig {
        app,
        project: dir.path().to_path_buf(),
        _dir: dir,
    };
    // Bodies are created and the character settles on the floor.
    rig.frames(30);
    rig
}

/// Replaces `from` with `to` in a project file (relative to `assets/`).
fn replace_in(assets: &Path, file: &str, from: &str, to: &str) {
    let path = assets.join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "`{from}` is not in {file}");
    std::fs::write(&path, text.replace(from, to)).unwrap();
}

impl Rig {
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.app.update();
            // Nothing else ends the frame for the input resource here.
            self.app.resources.fetch_mut::<Input>().mouse_delta = Vec2::ZERO;
        }
    }

    fn key(&self, key: KeyCode, down: bool) {
        self.app.resources.fetch_mut::<Input>().set_key(key, down);
    }

    /// One frame with the mouse moved by `delta` pixels.
    fn mouse(&mut self, delta: Vec2) {
        self.app.resources.fetch_mut::<Input>().mouse_delta = delta;
        self.app.update();
        self.app.resources.fetch_mut::<Input>().mouse_delta = Vec2::ZERO;
    }

    /// Holds `keys` for `n` frames, then lets go.
    fn hold(&mut self, keys: &[KeyCode], n: usize) {
        for k in keys {
            self.key(*k, true);
        }
        self.frames(n);
        for k in keys {
            self.key(*k, false);
        }
    }

    fn entity(&self, name: &str) -> Entity {
        let world = self.app.resources.fetch::<World>();
        world
            .query::<(Entity, &Name)>()
            .iter()
            .find(|(_, n)| n.0 == name)
            .map(|(e, _)| e)
            .unwrap_or_else(|| panic!("no entity `{name}`"))
    }

    /// Local transform.
    fn transform(&self, name: &str) -> Transform {
        let e = self.entity(name);
        *self
            .app
            .resources
            .fetch::<World>()
            .get::<&Transform>(e)
            .unwrap()
    }

    fn pos(&self, name: &str) -> Vec3 {
        self.transform(name).translation
    }

    /// World position (through `GlobalTransform`, after the last frame).
    fn world_pos(&self, name: &str) -> Vec3 {
        let e = self.entity(name);
        self.app
            .resources
            .fetch::<World>()
            .get::<&GlobalTransform>(e)
            .unwrap()
            .0
            .w_axis
            .truncate()
    }

    /// Yaw in degrees of an entity's local rotation.
    fn yaw(&self, name: &str) -> f32 {
        self.transform(name)
            .rotation
            .to_euler(EulerRot::YXZ)
            .0
            .to_degrees()
    }

    fn pitch(&self, name: &str) -> f32 {
        self.transform(name)
            .rotation
            .to_euler(EulerRot::YXZ)
            .1
            .to_degrees()
    }

    fn grounded(&self, name: &str) -> bool {
        let e = self.entity(name);
        self.app
            .resources
            .fetch::<World>()
            .get::<&CharacterMotor>(e)
            .unwrap()
            .grounded
    }
}

fn horizontal(v: Vec3) -> f32 {
    Vec3::new(v.x, 0.0, v.z).length()
}

/// Peak height of a jump started now (Space held for one frame), and whether
/// the character is back on the ground at the end.
fn jump_peak(rig: &mut Rig, who: &str) -> f32 {
    let start = rig.pos(who).y;
    rig.key(KeyCode::Space, true);
    rig.frames(1);
    rig.key(KeyCode::Space, false);
    let mut peak = start;
    for _ in 0..90 {
        rig.frames(1);
        peak = peak.max(rig.pos(who).y);
    }
    peak - start
}

// --- Pack contents ------------------------------------------------------------

#[test]
fn the_character_packs_are_well_formed() {
    let library = Library::embedded();
    assert!(library.problems().is_empty(), "{:?}", library.problems());
    for id in [
        "fps-controller",
        "third-person-controller",
        "top-down-controller",
    ] {
        let pack = library.get(id).unwrap_or_else(|| panic!("no pack {id}"));
        assert_eq!(pack.manifest.category, olvin_library::Category::Characters);
        assert!(pack.headline_prefab().is_some(), "{id}: a headline prefab");
        assert!(pack.demo_scene().is_some(), "{id}: a demo scene");
        assert!(pack.lint().is_empty(), "{id}: {:?}", pack.lint());
        assert!(!pack.manifest.notes.as_deref().unwrap_or("").is_empty());
        // Every RON asset parses with the engine's own readers.
        for f in &pack.files {
            if f.path.ends_with(".scene.ron") || f.path.ends_with(".prefab.ron") {
                let text = std::str::from_utf8(&f.data).unwrap();
                olvin::scene::file::SceneFile::parse(text)
                    .unwrap_or_else(|e| panic!("{id}/{}: {e}", f.path));
            }
        }
    }
}

// --- First person ---------------------------------------------------------------

#[test]
fn fps_the_player_is_a_character_with_a_camera_and_the_script() {
    let rig = rig("fps-controller", |_| {});
    let world = rig.app.resources.fetch::<World>();
    let player = rig.entity("Player");
    assert!(world.get::<&CharacterMotor>(player).is_ok());
    assert!(world.get::<&olvin::scene_file::Scripts>(player).is_ok());
    assert!(
        world.get::<&olvin::scene::PrefabInstanceOf>(player).is_ok(),
        "the player is an instance of the prefab"
    );
    drop(world);
    // The camera is the player's child, at eye height.
    assert!((rig.pos("Camera").y - 0.7).abs() < 0.01);
    assert!(rig.grounded("Player"), "settled on the floor");
    assert_clean_log();
}

#[test]
fn fps_w_walks_forward_and_shift_runs() {
    let mut rig = rig("fps-controller", |_| {});
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyW], 60);
    let walked = rig.pos("Player") - start;
    assert!(walked.z < -2.5, "forward is -Z, moved {walked}");
    assert!(walked.x.abs() < 0.05, "straight ahead, moved {walked}");
    assert!(walked.y.abs() < 0.05, "stays on the floor");

    // Back to the start, then run the same time.
    rig.frames(40);
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyW, KeyCode::ShiftLeft], 60);
    let ran = rig.pos("Player") - start;
    assert!(
        horizontal(ran) > horizontal(walked) * 1.4,
        "sprint is faster: {} vs {}",
        horizontal(ran),
        horizontal(walked)
    );
    assert_clean_log();
}

#[test]
fn fps_strafes_and_walks_backwards() {
    let mut rig = rig("fps-controller", |_| {});
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyD], 45);
    assert!(
        (rig.pos("Player") - start).x > 1.5,
        "D is to the right (+X)"
    );
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyS], 45);
    assert!((rig.pos("Player") - start).z > 1.5, "S walks back (+Z)");
    assert_clean_log();
}

#[test]
fn fps_diagonal_is_not_faster() {
    let mut rig = rig("fps-controller", |_| {});
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyW], 90);
    let straight = horizontal(rig.pos("Player") - start);
    rig.frames(30);
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyA, KeyCode::KeyW], 90);
    let diagonal = horizontal(rig.pos("Player") - start);
    assert!(diagonal < straight * 1.1, "{diagonal} vs {straight}");
    assert!(diagonal > straight * 0.8);
    assert_clean_log();
}

#[test]
fn fps_space_jumps_about_jump_height() {
    let mut rig = rig("fps-controller", |_| {});
    let peak = jump_peak(&mut rig, "Player");
    // jump_height is 1.2 m; fixed-step integration is a little off.
    assert!((0.9..1.5).contains(&peak), "peak {peak}");
    assert!(rig.grounded("Player"), "landed again");

    // No double jump in the air.
    rig.key(KeyCode::Space, true);
    rig.frames(1);
    rig.key(KeyCode::Space, false);
    rig.frames(20);
    let mid = rig.pos("Player").y;
    rig.key(KeyCode::Space, true);
    rig.frames(1);
    rig.key(KeyCode::Space, false);
    rig.frames(5);
    assert!(rig.pos("Player").y < mid + 0.5, "a second jump in the air");
    assert_clean_log();
}

#[test]
fn fps_mouse_turns_the_body_and_tilts_the_camera() {
    let mut rig = rig("fps-controller", |_| {});
    // Moving the mouse left (negative x) turns left (positive yaw): 100 px * 0.12 deg.
    rig.mouse(Vec2::new(-100.0, 0.0));
    let yaw = rig.yaw("Player");
    assert!((yaw - 12.0).abs() < 0.5, "yaw {yaw}");
    // The mouse moves up the screen (negative y): look up.
    rig.mouse(Vec2::new(0.0, -100.0));
    let pitch = rig.pitch("Camera");
    assert!((pitch - 12.0).abs() < 0.5, "pitch {pitch}");
    assert!(rig.yaw("Camera").abs() < 0.5, "the camera only tilts");
    // The pitch is clamped at max_pitch.
    for _ in 0..20 {
        rig.mouse(Vec2::new(0.0, -500.0));
    }
    assert!((rig.pitch("Camera") - 85.0).abs() < 0.5);
    for _ in 0..40 {
        rig.mouse(Vec2::new(0.0, 500.0));
    }
    assert!((rig.pitch("Camera") + 85.0).abs() < 0.5);

    // Forward follows where the body faces: turn 90 degrees left, walk.
    rig.mouse(Vec2::new(-750.0, 0.0));
    assert!((rig.yaw("Player") - rig.yaw("Player").round()).abs() < 1.0);
    let yaw = rig.yaw("Player").to_radians();
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyW], 60);
    let moved = rig.pos("Player") - start;
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    assert!(
        moved.normalize().dot(forward) > 0.98,
        "moved {moved}, facing {forward}"
    );
    assert_clean_log();
}

#[test]
fn fps_parameters_from_the_prefab_are_used() {
    let mut rig = rig("fps-controller", |assets| {
        replace_in(
            assets,
            "prefabs/fps-controller/Player.prefab.ron",
            "\"speed\": \"4.0\"",
            "\"speed\": \"8.0\"",
        );
        replace_in(
            assets,
            "prefabs/fps-controller/Player.prefab.ron",
            "\"jump_height\": \"1.2\"",
            "\"jump_height\": \"0.5\"",
        );
    });
    let start = rig.pos("Player");
    rig.hold(&[KeyCode::KeyW], 60);
    assert!(horizontal(rig.pos("Player") - start) > 5.0, "speed 8 m/s");
    rig.frames(30);
    let peak = jump_peak(&mut rig, "Player");
    assert!((0.3..0.75).contains(&peak), "peak {peak}");
    assert_clean_log();
}

#[test]
fn fps_crouch_lowers_the_camera_when_the_action_exists() {
    let mut rig = rig("fps-controller", |_| {});
    let standing = rig.pos("Camera").y;
    // Without a Crouch action, nothing happens.
    rig.key(KeyCode::ControlLeft, true);
    rig.frames(30);
    assert!((rig.pos("Camera").y - standing).abs() < 0.01);
    rig.key(KeyCode::ControlLeft, false);
    // The project adds the action (assets/input.ron does this for a real game).
    rig.app
        .resources
        .fetch_mut::<InputMap>()
        .bind("Crouch", [Binding::Key(KeyCode::ControlLeft)]);
    rig.key(KeyCode::ControlLeft, true);
    rig.frames(60);
    let crouched = rig.pos("Camera").y;
    assert!(standing - crouched > 0.4, "{standing} -> {crouched}");
    rig.key(KeyCode::ControlLeft, false);
    rig.frames(60);
    assert!(
        (rig.pos("Camera").y - standing).abs() < 0.02,
        "stands up again"
    );
    assert_clean_log();
}

#[test]
fn fps_head_bob_moves_the_camera_while_walking() {
    let mut rig = rig("fps-controller", |assets| {
        replace_in(
            assets,
            "prefabs/fps-controller/Player.prefab.ron",
            "\"head_bob\": \"0.0\"",
            "\"head_bob\": \"0.05\"",
        );
    });
    rig.key(KeyCode::KeyW, true);
    rig.frames(20);
    let ys: Vec<f32> = (0..60)
        .map(|_| {
            rig.frames(1);
            rig.pos("Camera").y
        })
        .collect();
    let (lo, hi) = ys
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(*y), hi.max(*y)));
    assert!(hi - lo > 0.03, "camera bob {lo}..{hi}");
    assert_clean_log();
}

// --- Third person -----------------------------------------------------------------

#[test]
fn tps_walks_relative_to_the_camera_and_the_body_turns() {
    let mut rig = rig("third-person-controller", |_| {});
    assert!(rig.grounded("Character"));
    // The camera orbits at `distance` behind and above the character.
    let to_cam = rig.world_pos("Camera") - rig.world_pos("Character");
    assert!((to_cam.length() - 5.0).abs() < 1.0, "{to_cam}");
    assert!(
        to_cam.z > 3.0 && to_cam.y > 0.5,
        "behind and above: {to_cam}"
    );

    let start = rig.pos("Character");
    rig.hold(&[KeyCode::KeyW], 60);
    let moved = rig.pos("Character") - start;
    assert!(
        moved.z < -2.5 && moved.x.abs() < 0.1,
        "W is away from the camera: {moved}"
    );
    // The body faces -Z (yaw 0), the way it walks.
    assert!(rig.yaw("Body").abs() < 5.0, "body yaw {}", rig.yaw("Body"));

    // D: the body turns to face +X (yaw -90) smoothly, not in one frame.
    rig.key(KeyCode::KeyD, true);
    rig.frames(2);
    let early = rig.yaw("Body");
    assert!(
        early < -1.0 && early > -80.0,
        "turning, not snapped: {early}"
    );
    rig.frames(60);
    rig.key(KeyCode::KeyD, false);
    assert!(
        (rig.yaw("Body") + 90.0).abs() < 3.0,
        "body yaw {}",
        rig.yaw("Body")
    );
    assert!(rig.pos("Character").x - start.x > 2.0);
    // The root itself never turns.
    assert!(rig.yaw("Character").abs() < 0.01);
    assert_clean_log();
}

#[test]
fn tps_the_mouse_orbits_the_camera_and_w_follows_it() {
    let mut rig = rig("third-person-controller", |_| {});
    // Orbit a quarter turn (-750 px * 0.12 = +90 degrees of camera yaw): the
    // camera ends up east of the character, looking along -X.
    rig.mouse(Vec2::new(-750.0, 0.0));
    rig.frames(2);
    assert!(
        (rig.yaw("Camera") - 90.0).abs() < 1.0,
        "camera yaw {}",
        rig.yaw("Camera")
    );
    let to_cam = rig.world_pos("Camera") - rig.world_pos("Character");
    assert!(
        to_cam.x > 3.0 && to_cam.z.abs() < 1.5,
        "camera east of the character: {to_cam}"
    );
    let start = rig.pos("Character");
    rig.hold(&[KeyCode::KeyW], 60);
    let moved = rig.pos("Character") - start;
    assert!(
        moved.x < -2.5 && moved.z.abs() < 0.3,
        "W follows the camera: {moved}"
    );
    // The camera keeps its distance while the character moves.
    let to_cam = rig.world_pos("Camera") - rig.world_pos("Character");
    assert!((to_cam.length() - 5.0).abs() < 1.0);
    // Vertical orbit is clamped.
    for _ in 0..30 {
        rig.mouse(Vec2::new(0.0, -500.0));
    }
    assert!((rig.pitch("Camera") - 25.0).abs() < 0.5);
    for _ in 0..60 {
        rig.mouse(Vec2::new(0.0, 500.0));
    }
    assert!((rig.pitch("Camera") + 55.0).abs() < 0.5);
    assert_clean_log();
}

#[test]
fn tps_jump_and_sprint() {
    let mut rig = rig("third-person-controller", |_| {});
    let peak = jump_peak(&mut rig, "Character");
    assert!((1.0..1.6).contains(&peak), "peak {peak} (jump_height 1.3)");
    assert!(rig.grounded("Character"));

    let start = rig.pos("Character");
    rig.hold(&[KeyCode::KeyW], 60);
    let walked = horizontal(rig.pos("Character") - start);
    rig.frames(40);
    let start = rig.pos("Character");
    rig.hold(&[KeyCode::KeyW, KeyCode::ShiftLeft], 60);
    let ran = horizontal(rig.pos("Character") - start);
    assert!(ran > walked * 1.3, "{ran} vs {walked}");
    assert_clean_log();
}

#[test]
fn tps_camera_follow_works_on_a_free_camera() {
    let mut rig = rig("third-person-controller", |assets| {
        // A free camera that follows the character by name, next to the
        // prefab's own camera child.
        let scene = assets.join("scenes/third-person-controller/demo.scene.ron");
        let text = std::fs::read_to_string(&scene).unwrap();
        let free = r#"        (id: 200, name: "Free Camera", position: (0.0, 4.0, 14.0), rotation: (0.0, 0.0, 0.0), scale: (1.0, 1.0, 1.0),
            camera: Some((fov_deg: 60.0, near: 0.1, far: 100.0)),
            scripts: [(name: "third-person-controller/camera_follow", params: {"target": "Character", "distance": "7.0", "height": "1.0", "smoothing": "8.0"})]),
    ],
)"#;
        let end = text.rfind("    ],\n)").unwrap();
        std::fs::write(&scene, format!("{}{free}\n", &text[..end])).unwrap();
    });
    rig.frames(90);
    let to_cam = rig.world_pos("Free Camera") - rig.world_pos("Character");
    // Pivot is 1 m above the character: distance 7 from there.
    let from_pivot = to_cam - Vec3::Y;
    assert!((from_pivot.length() - 7.0).abs() < 0.3, "{to_cam}");
    // It follows when the character walks away (the camera is behind, so W
    // leads away from it, and it catches up).
    rig.hold(&[KeyCode::KeyW], 120);
    rig.frames(120);
    let to_cam = rig.world_pos("Free Camera") - rig.world_pos("Character");
    assert!(((to_cam - Vec3::Y).length() - 7.0).abs() < 0.3, "{to_cam}");
    assert_clean_log();
}

// --- Top down --------------------------------------------------------------------

#[test]
fn top_down_walks_in_eight_directions_at_one_speed() {
    let mut rig = rig("top-down-controller", |_| {});
    assert!(rig.grounded("Character"));
    let run = |rig: &mut Rig, keys: &[KeyCode]| {
        rig.frames(20);
        let start = rig.pos("Character");
        rig.hold(keys, 40);
        rig.pos("Character") - start
    };
    let up = run(&mut rig, &[KeyCode::KeyW]);
    assert!(up.z < -2.0 && up.x.abs() < 0.05, "W is -Z: {up}");
    let right = run(&mut rig, &[KeyCode::KeyD]);
    assert!(right.x > 2.0 && right.z.abs() < 0.05, "D is +X: {right}");
    let diagonal = run(&mut rig, &[KeyCode::KeyW, KeyCode::KeyD]);
    assert!(diagonal.x > 1.0 && diagonal.z < -1.0, "{diagonal}");
    assert!(
        (diagonal.x + diagonal.z).abs() < 0.1,
        "a true 45 degree line: {diagonal}"
    );
    assert!(
        (horizontal(diagonal) - horizontal(up)).abs() < 0.2,
        "diagonals are not faster: {} vs {}",
        horizontal(diagonal),
        horizontal(up)
    );
    assert_clean_log();
}

#[test]
fn top_down_body_faces_the_walking_direction_and_the_camera_follows() {
    let mut rig = rig("top-down-controller", |_| {});
    let cam_offset = rig.world_pos("Camera") - rig.world_pos("Character");
    rig.hold(&[KeyCode::KeyD], 45);
    assert!(
        (rig.yaw("Body") + 90.0).abs() < 3.0,
        "faces +X: {}",
        rig.yaw("Body")
    );
    rig.hold(&[KeyCode::KeyS], 45);
    assert!(
        rig.yaw("Body").abs() > 177.0,
        "faces +Z: {}",
        rig.yaw("Body")
    );
    // The camera is parented: same offset, wherever the character is.
    rig.frames(2);
    let now = rig.world_pos("Camera") - rig.world_pos("Character");
    assert!((now - cam_offset).length() < 0.01);
    assert!(cam_offset.y > 8.0, "above: {cam_offset}");
    // No jumping by default.
    let peak = jump_peak(&mut rig, "Character");
    assert!(peak < 0.1, "jumped {peak}");
    assert_clean_log();
}

#[test]
fn top_down_isometric_and_jump_options() {
    let mut rig = rig("top-down-controller", |assets| {
        let prefab = "prefabs/top-down-controller/Character.prefab.ron";
        replace_in(
            assets,
            prefab,
            "\"camera_yaw\": \"0.0\"",
            "\"camera_yaw\": \"45.0\"",
        );
        replace_in(
            assets,
            prefab,
            "\"jump_height\": \"0.0\"",
            "\"jump_height\": \"1.0\"",
        );
        replace_in(
            assets,
            prefab,
            "\"snap_to_8\": \"false\"",
            "\"snap_to_8\": \"true\"",
        );
    });
    let start = rig.pos("Character");
    rig.hold(&[KeyCode::KeyW], 40);
    let moved = rig.pos("Character") - start;
    // Screen-up with the view turned 45 degrees: toward -X and -Z.
    assert!(moved.x < -1.0 && moved.z < -1.0, "{moved}");
    assert!((moved.x - moved.z).abs() < 0.1, "{moved}");
    // Snapped to a compass direction: 45 degrees.
    assert!(
        (rig.yaw("Body") - 45.0).abs() < 3.0,
        "body yaw {}",
        rig.yaw("Body")
    );
    rig.frames(30);
    let peak = jump_peak(&mut rig, "Character");
    assert!((0.7..1.3).contains(&peak), "peak {peak}");
    assert_clean_log();
}

// --- Importing -------------------------------------------------------------------

#[test]
fn importing_creates_metas_and_a_lock() {
    let rig = rig("fps-controller", |_| {});
    let assets = rig.project.join("assets");
    assert!(
        assets
            .join("scripts/fps-controller/player.lua.meta")
            .is_file()
    );
    assert!(
        assets
            .join("prefabs/fps-controller/Player.prefab.ron.meta")
            .is_file()
    );
    let installed = olvin_library::installed(&rig.project).unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].id, "fps-controller");
    assert_eq!(installed[0].files.len(), 3);
}
