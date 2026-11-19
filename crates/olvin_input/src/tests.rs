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

fn rebind_frame(rebind: &mut Rebind, map: &mut InputMap, input: &mut Input, pads: &GamepadState) {
    rebind.poll(map, input, pads);
    input.end_frame();
}

#[test]
fn rebind_replaces_adds_and_reports_conflicts() {
    let mut map = InputMap::default();
    let (mut input, pads, mut rebind) =
        (Input::default(), GamepadState::default(), Rebind::default());
    rebind.listen("Jump", Slot::Replace(0));
    // Nothing pressed yet: still listening.
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert!(rebind.is_listening());
    // E is Interact's key, so it is reported as a conflict, not refused.
    input.set_key(KeyCode::KeyE, true);
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert!(!rebind.is_listening());
    assert_eq!(map.actions["Jump"][0], Binding::Key(KeyCode::KeyE));
    assert_eq!(
        rebind.take_event(),
        Some(RebindEvent::Bound {
            action: "Jump".into(),
            binding: Binding::Key(KeyCode::KeyE),
            conflicts: vec![("Interact".into(), 0)],
        })
    );
    assert_eq!(rebind.take_event(), None);

    // A held key is not a new press; a mouse button is.
    rebind.listen("Jump", Slot::Add);
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert!(rebind.is_listening());
    input.set_button(MouseButton::Right, true);
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert_eq!(map.actions["Jump"].len(), 3);
    assert_eq!(map.actions["Jump"][2], Binding::Mouse(MouseButton::Right));
}

#[test]
fn rebind_takes_pad_presses_and_escape_cancels() {
    let mut map = InputMap::default();
    let before = map.clone();
    let (mut input, mut pads, mut rebind) =
        (Input::default(), GamepadState::default(), Rebind::default());
    // Held before listening does not count.
    pads.set_button(GamepadButton::North, true);
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    rebind.listen("Fire", Slot::Replace(1));
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert!(rebind.is_listening());
    pads.set_button(GamepadButton::East, true);
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert_eq!(
        map.actions["Fire"][1],
        Binding::GamepadButton(GamepadButton::East)
    );

    let mut map2 = before.clone();
    rebind.listen("Fire", Slot::Replace(0));
    input.set_key(KeyCode::Escape, true);
    rebind_frame(&mut rebind, &mut map2, &mut input, &pads);
    assert_eq!(map2, before);
    assert_eq!(rebind.take_event(), Some(RebindEvent::Cancelled));
}

#[test]
fn map_edit_helpers() {
    let defaults = InputMap::default();
    let mut map = defaults.clone();
    map.set_binding("Jump", 5, Binding::Key(KeyCode::KeyJ));
    assert_eq!(map.actions["Jump"].len(), 3);
    assert!(map.remove_binding("Jump", 0));
    assert!(!map.remove_binding("Jump", 9) && !map.remove_binding("Nope", 0));
    map.reset_action("Jump", &defaults);
    assert_eq!(map.actions["Jump"], defaults.actions["Jump"]);
    map.bind("Custom", [Binding::Key(KeyCode::KeyJ)]);
    map.reset_action("Custom", &defaults);
    assert!(!map.actions.contains_key("Custom"));
    let mut old = InputMap::empty();
    old.bind("Jump", [Binding::Key(KeyCode::KeyJ)]);
    old.fill_missing(&defaults);
    assert_eq!(old.actions["Jump"], [Binding::Key(KeyCode::KeyJ)]);
    assert_eq!(old.actions["Move"], defaults.actions["Move"]);
}

#[test]
fn rebind_survives_save_and_autosaves() {
    let dir = std::env::temp_dir().join(format!("olvin-rebind-{}", std::process::id()));
    let path = dir.join("user.ron");
    let mut map = InputMap::default();
    let (mut input, pads) = (Input::default(), GamepadState::default());
    let mut rebind = Rebind::default();
    rebind.autosave = Some(path.clone());
    rebind.listen("Jump", Slot::Replace(0));
    input.set_key(KeyCode::KeyJ, true);
    rebind_frame(&mut rebind, &mut map, &mut input, &pads);
    assert_eq!(InputMap::load(&path).unwrap(), map);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn plugin_listening_swallows_the_press() {
    let mut app = App::new();
    app.add_plugin(InputPlugin {
        map: Some(InputMap::default()),
    });
    app.resources
        .fetch_mut::<Rebind>()
        .listen("Jump", Slot::Replace(0));
    app.resources
        .fetch_mut::<Input>()
        .set_key(KeyCode::KeyE, true);
    app.update();
    // E became Jump's key; Interact (also E) did not fire this frame.
    assert!(!app.resources.fetch::<ActionState>().pressed("Interact"));
    assert_eq!(
        app.resources.fetch::<InputMap>().actions["Jump"][0],
        Binding::Key(KeyCode::KeyE)
    );
    app.update();
    assert!(app.resources.fetch::<ActionState>().pressed("Jump"));
}

#[test]
fn hints_follow_device_and_pad_style() {
    let map = InputMap::default();
    let xbox = PadStyle::Xbox;
    assert_eq!(
        map.hint("Jump", Device::KeyboardMouse, xbox).unwrap(),
        "Space"
    );
    assert_eq!(map.hint("Jump", Device::Gamepad, xbox).unwrap(), "A");
    assert_eq!(
        map.hint("Jump", Device::Gamepad, PadStyle::PlayStation)
            .unwrap(),
        "Cross"
    );
    assert_eq!(
        map.hint("Jump", Device::Gamepad, PadStyle::Nintendo)
            .unwrap(),
        "B"
    );
    assert_eq!(
        map.hint("Pause", Device::Gamepad, PadStyle::PlayStation)
            .unwrap(),
        "Options"
    );
    assert_eq!(
        map.hint("Pause", Device::KeyboardMouse, xbox).unwrap(),
        "Esc"
    );
    assert_eq!(
        map.hints("Move", Device::KeyboardMouse, xbox),
        ["W/A/S/D", "Up/Left/Down/Right"]
    );
    assert_eq!(
        map.hint("Move", Device::Gamepad, xbox).unwrap(),
        "Left Stick"
    );
    assert_eq!(
        map.hint("Fire", Device::KeyboardMouse, xbox).unwrap(),
        "Left Click"
    );
    assert_eq!(map.hint("Nope", Device::Gamepad, xbox), None);
    let chord = Binding::chord([KeyCode::ControlLeft], Binding::Key(KeyCode::KeyS));
    assert_eq!(chord.label(xbox), "Left Ctrl+S");
    assert_eq!(chord.device(), Some(Device::KeyboardMouse));
    let mixed = Binding::Axis1D {
        negative: Box::new(Binding::Key(KeyCode::KeyA)),
        positive: Box::new(Binding::GamepadButton(GamepadButton::South)),
    };
    assert_eq!(mixed.device(), None);
    assert_eq!(key_label(KeyCode::Digit1), "1");
    assert_eq!(key_label(KeyCode::PageUp), "Page Up");
    assert_eq!(key_label(KeyCode::Numpad5), "Numpad 5");
    assert_eq!(key_label(KeyCode::F11), "F11");
}

#[test]
fn pad_style_detection() {
    assert_eq!(PadStyle::detect(Some(0x054c), "x"), PadStyle::PlayStation);
    assert_eq!(PadStyle::detect(Some(0x057e), "x"), PadStyle::Nintendo);
    assert_eq!(
        PadStyle::detect(None, "Sony DualSense"),
        PadStyle::PlayStation
    );
    assert_eq!(PadStyle::detect(None, "Some Pad"), PadStyle::Xbox);
}

#[test]
fn last_device_follows_use() {
    let map = InputMap::default();
    let (mut input, mut pads, mut state) = (
        Input::default(),
        GamepadState::default(),
        ActionState::default(),
    );
    assert_eq!(state.device, Device::KeyboardMouse);
    pads.set_button(GamepadButton::South, true);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.device, Device::Gamepad);
    pads.set_button(GamepadButton::South, false);
    input.set_key(KeyCode::KeyW, true);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.device, Device::KeyboardMouse);
    // Idle frames change nothing.
    input.set_key(KeyCode::KeyW, false);
    frame(&mut state, &map, &mut input, &pads);
    assert_eq!(state.device, Device::KeyboardMouse);
}

#[test]
fn haptics_queue_scales_and_stops() {
    use std::time::Duration;
    let mut h = Haptics::default();
    h.rumble(2.0, -1.0, Duration::from_millis(100));
    h.pulse(0.5, Duration::from_millis(50));
    h.intensity = 0.5;
    let (list, stop) = h.take();
    assert!(!stop);
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].strong, list[0].weak), (0.5, 0.0));
    assert_eq!(list[1].strong, 0.25);
    assert!(h.take().0.is_empty());
    h.pulse(1.0, Duration::from_secs(1));
    h.stop();
    let (list, stop) = h.take();
    assert!(list.is_empty() && stop);
    h.enabled = false;
    h.pulse(1.0, Duration::from_secs(1));
    let (list, stop) = h.take();
    assert!(
        list.is_empty() && stop,
        "disabled drops requests and cuts effects"
    );
}
