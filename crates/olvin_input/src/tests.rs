//! Driven by synthetic `Input` state only: no device, no window.

use glam::Vec2;
use olvin_core::App;

use super::*;

fn frame(state: &mut ActionState, map: &InputMap, input: &mut Input, pads: &GamepadState) {
    state.update(map, input, pads);
    input.end_frame();
}

#[test]
fn wasd_composite_is_normalised_and_arrows_also_work() {
    let map = InputMap::default();
    let (mut input, pads, mut state) = (
        Input::default(),
        GamepadState::default(),
        ActionState::default(),
    );
    input.set_key(KeyCode::KeyW, true);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.axis2d("Move"), Vec2::Y);
    input.set_key(KeyCode::KeyD, true);
    frame(&mut state, &map, &mut input, &pads);
    let v = state.axis2d("Move");
    assert!(
        (v.length() - 1.0).abs() < 1e-5 && v.x > 0.7 && v.y > 0.7,
        "{v}"
    );
    input.set_key(KeyCode::KeyW, false);
    input.set_key(KeyCode::KeyD, false);
    input.set_key(KeyCode::ArrowLeft, true);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.axis2d("Move"), Vec2::NEG_X);
    // Opposite keys cancel.
    input.set_key(KeyCode::ArrowRight, true);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.axis2d("Move"), Vec2::ZERO);
}

#[test]
fn just_pressed_and_released_last_one_frame() {
    let map = InputMap::default();
    let (mut input, pads, mut state) = (
        Input::default(),
        GamepadState::default(),
        ActionState::default(),
    );
    frame(&mut state, &map, &mut input, &pads);
    assert!(!state.pressed("Jump"));

    input.set_key(KeyCode::Space, true);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.pressed("Jump") && state.just_pressed("Jump"));
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.pressed("Jump") && !state.just_pressed("Jump"));

    input.set_key(KeyCode::Space, false);
    frame(&mut state, &map, &mut input, &pads);
    assert!(!state.pressed("Jump") && state.just_released("Jump"));
    frame(&mut state, &map, &mut input, &pads);
    assert!(!state.just_released("Jump"));

    // A tap within one frame still fires once.
    input.set_key(KeyCode::Space, true);
    input.set_key(KeyCode::Space, false);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.just_pressed("Jump"));
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.just_released("Jump"));

    // Mouse buttons and unknown actions.
    input.set_button(MouseButton::Left, true);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.just_pressed("Fire"));
    assert!(!state.pressed("NoSuchAction"));
}

#[test]
fn gamepad_sticks_buttons_and_deadzone() {
    let map = InputMap::default();
    let (mut input, mut pads, mut state) = (
        Input::default(),
        GamepadState::default(),
        ActionState::default(),
    );
    pads.set_axis(GamepadAxis::LeftStickX, 0.1);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.axis2d("Move"), Vec2::ZERO, "inside the dead zone");
    pads.set_axis(GamepadAxis::LeftStickX, 1.0);
    pads.set_axis(GamepadAxis::LeftStickY, 0.0);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.axis2d("Move").abs_diff_eq(Vec2::X, 1e-5));
    pads.set_button(GamepadButton::South, true);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.just_pressed("Jump"));
    // The strongest binding wins: keyboard half-pressed vs stick.
    pads.set_axis(GamepadAxis::LeftStickX, 0.0);
    pads.set_axis(GamepadAxis::RightStickY, 1.0);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.axis2d("Look").y > 10.0, "stick look is scaled");
}

#[test]
fn mouse_motion_scroll_and_chords() {
    let mut map = InputMap::empty();
    map.bind("Look", [Binding::MouseMotion { scale: 0.5 }])
        .bind("Zoom", [Binding::Scroll { scale: 1.0 }])
        .bind(
            "Save",
            [Binding::chord(
                [KeyCode::ControlLeft],
                Binding::Key(KeyCode::KeyS),
            )],
        )
        .bind("Lean", [Binding::keys_1d(KeyCode::KeyQ, KeyCode::KeyE)]);
    let (mut input, pads, mut state) = (
        Input::default(),
        GamepadState::default(),
        ActionState::default(),
    );
    input.mouse_delta = Vec2::new(10.0, 4.0);
    input.scroll = Vec2::new(0.0, -2.0);
    input.set_key(KeyCode::KeyS, true);
    input.set_key(KeyCode::KeyQ, true);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.axis2d("Look"), Vec2::new(5.0, -2.0), "+Y is up");
    assert_eq!(state.axis2d("Zoom").y, -2.0);
    assert_eq!(state.mouse_delta, Vec2::new(10.0, 4.0));
    assert!(!state.pressed("Save"), "S alone is not Ctrl+S");
    assert_eq!(state.value("Lean"), -1.0);
    input.set_key(KeyCode::ControlLeft, true);
    frame(&mut state, &map, &mut input, &pads);
    assert!(state.just_pressed("Save"));
    assert_eq!(state.axis2d("Look"), Vec2::ZERO, "deltas are per frame");
}

#[test]
fn ron_round_trip_and_file() {
    let map = InputMap::default();
    let text = map.to_ron();
    assert!(text.contains("KeyW") && text.contains("Axis2D"), "{text}");
    assert_eq!(InputMap::from_ron(&text).unwrap(), map);

    let hand_written = r#"(
        actions: {
            "Jump": [Key(Space), GamepadButton(South)],
            "Move": [Axis2D(up: Key(KeyW), down: Key(KeyS), left: Key(KeyA), right: Key(KeyD))],
            "Look": [MouseMotion(), GamepadStick(stick: Right, scale: 8.0)],
            "Aim": [Mouse(Right), GamepadAxis(axis: LeftTrigger)],
        },
    )"#;
    let parsed = InputMap::from_ron(hand_written).unwrap();
    assert_eq!(parsed.deadzone, 0.15, "defaults fill missing fields");
    assert_eq!(
        parsed.actions["Look"][0],
        Binding::MouseMotion { scale: 1.0 }
    );

    let dir = std::env::temp_dir().join(format!("olvin-input-{}", std::process::id()));
    let path = dir.join("input.ron");
    parsed.save(&path).unwrap();
    assert_eq!(InputMap::load(&path).unwrap(), parsed);
    assert_eq!(
        InputMap::load_or_default(dir.join("missing.ron")),
        InputMap::default()
    );
    std::fs::write(&path, "not ron").unwrap();
    assert_eq!(InputMap::load_or_default(&path), InputMap::default());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn plugin_updates_actions_and_cursor_headless() {
    let mut app = App::new();
    app.insert_resource(CursorOptions::default());
    app.add_plugin(InputPlugin {
        map: Some(InputMap::default()),
    });
    app.resources
        .fetch_mut::<Input>()
        .set_key(KeyCode::KeyE, true);
    app.update();
    assert!(
        app.resources
            .fetch::<ActionState>()
            .just_pressed("Interact")
    );
    app.resources.fetch_mut::<ActionState>().lock_cursor(true);
    app.update();
    let cursor = *app.resources.fetch::<CursorOptions>();
    assert!(cursor.grabbed && !cursor.visible);
}
