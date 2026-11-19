//! Gamepad state merged over every connected pad, and the gilrs backend.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::{GamepadAxis, GamepadButton, PadStyle};

/// Buttons and axes of all connected gamepads merged (any pad can play).
/// Filled by gilrs with the `gamepad` feature; tests and other backends can
/// write it directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GamepadState {
    buttons: HashSet<GamepadButton>,
    axes: HashMap<GamepadAxis, f32>,
    /// Connected gamepads.
    pub connected: usize,
    /// Button naming of the first connected pad (the last one seen once it is gone).
    pub style: PadStyle,
}

impl GamepadState {
    pub fn pressed(&self, button: GamepadButton) -> bool {
        self.buttons.contains(&button)
    }

    pub fn axis(&self, axis: GamepadAxis) -> f32 {
        self.axes.get(&axis).copied().unwrap_or(0.0)
    }

    pub fn set_button(&mut self, button: GamepadButton, down: bool) {
        if down {
            self.buttons.insert(button);
        } else {
            self.buttons.remove(&button);
        }
    }

    pub fn set_axis(&mut self, axis: GamepadAxis, value: f32) {
        self.axes.insert(axis, value.clamp(-1.0, 1.0));
    }

    pub fn clear(&mut self) {
        self.buttons.clear();
        self.axes.clear();
        self.connected = 0;
    }
}

/// One vibration request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rumble {
    /// Heavy (low-frequency) motor, 0..1.
    pub strong: f32,
    /// Light (high-frequency) motor, 0..1.
    pub weak: f32,
    pub duration: Duration,
}

/// Vibration for gamepads: gameplay queues requests, the backend plays them
/// on every pad that supports force feedback. Without the `gamepad` feature
/// (or without a pad) the queue is just dropped each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Haptics {
    /// Master switch (a "vibration" option); off drops requests.
    pub enabled: bool,
    /// Scales every request, 0..1.
    pub intensity: f32,
    /// True when a connected pad can vibrate (set by the backend).
    pub supported: bool,
    queue: Vec<Rumble>,
    stop: bool,
}

impl Default for Haptics {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 1.0,
            supported: false,
            queue: Vec::new(),
            stop: false,
        }
    }
}

impl Haptics {
    /// Vibrates for `duration`; motor strengths are clamped to 0..1.
    pub fn rumble(&mut self, strong: f32, weak: f32, duration: Duration) {
        self.queue.push(Rumble {
            strong: strong.clamp(0.0, 1.0),
            weak: weak.clamp(0.0, 1.0),
            duration,
        });
    }

    /// Both motors at one strength.
    pub fn pulse(&mut self, strength: f32, duration: Duration) {
        self.rumble(strength, strength, duration);
    }

    /// Cuts everything now, including requests not played yet.
    pub fn stop(&mut self) {
        self.queue.clear();
        self.stop = true;
    }

    /// What the backend should play now, scaled by `intensity`; empties the queue.
    /// Returns the requests and whether playing effects must be stopped first.
    pub fn take(&mut self) -> (Vec<Rumble>, bool) {
        let stop = std::mem::take(&mut self.stop) || !self.enabled;
        let scale = if self.enabled {
            self.intensity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let list = std::mem::take(&mut self.queue)
            .into_iter()
            .filter(|_| scale > 0.0)
            .map(|r| Rumble {
                strong: r.strong * scale,
                weak: r.weak * scale,
                ..r
            })
            .collect();
        (list, stop)
    }
}

#[cfg(feature = "gamepad")]
pub(crate) mod backend {
    use std::time::Instant;

    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
    use gilrs::{Axis, Button, Gilrs};

    use super::*;

    /// The gilrs context; `None` when no gamepad subsystem is available.
    pub(crate) struct Gamepads {
        pub gilrs: Option<Gilrs>,
        /// Playing effects; dropping one stops it.
        playing: Vec<(gilrs::ff::Effect, Instant)>,
    }

    impl Gamepads {
        pub(crate) fn new() -> Self {
            let gilrs = match Gilrs::new() {
                Ok(g) => Some(g),
                Err(gilrs::Error::NotImplemented(g)) => {
                    log::info!("gamepads are not supported on this platform");
                    Some(g)
                }
                Err(err) => {
                    log::warn!("gamepads unavailable: {err}");
                    None
                }
            };
            Self {
                gilrs,
                playing: Vec::new(),
            }
        }

        fn rumble(&mut self, r: Rumble) {
            let Some(gilrs) = &mut self.gilrs else { return };
            let ids: Vec<_> = gilrs
                .gamepads()
                .filter(|(_, pad)| pad.is_ff_supported())
                .map(|(id, _)| id)
                .collect();
            if ids.is_empty() {
                return;
            }
            let replay = Replay {
                play_for: Ticks::from_ms(r.duration.as_millis().min(u32::MAX as u128) as u32),
                ..Default::default()
            };
            let motor = |kind| BaseEffect {
                kind,
                scheduling: replay,
                ..Default::default()
            };
            let to_u16 = |v: f32| (v * u16::MAX as f32) as u16;
            let effect = EffectBuilder::new()
                .add_effect(motor(BaseEffectType::Strong {
                    magnitude: to_u16(r.strong),
                }))
                .add_effect(motor(BaseEffectType::Weak {
                    magnitude: to_u16(r.weak),
                }))
                .gamepads(&ids)
                .finish(gilrs)
                .and_then(|e| e.play().map(|()| e));
            match effect {
                Ok(effect) => self.playing.push((effect, Instant::now() + r.duration)),
                Err(err) => log::warn!("gamepad rumble: {err}"),
            }
        }

        /// Drains events and rebuilds the merged state.
        pub(crate) fn poll(&mut self, state: &mut GamepadState, haptics: &mut Haptics) {
            let (requests, stop) = haptics.take();
            if stop {
                self.playing.clear();
            }
            let now = Instant::now();
            self.playing.retain(|(_, end)| *end > now);
            for r in requests {
                self.rumble(r);
            }
            let Some(gilrs) = &mut self.gilrs else { return };
            while gilrs.next_event().is_some() {}
            state.clear();
            haptics.supported = false;
            for (_, pad) in gilrs.gamepads() {
                if state.connected == 0 {
                    state.style = PadStyle::detect(pad.vendor_id(), pad.name());
                }
                state.connected += 1;
                haptics.supported |= pad.is_ff_supported();
                for b in GamepadButton::ALL {
                    if pad.is_pressed(button(b)) {
                        state.set_button(b, true);
                    }
                }
                for a in GamepadAxis::ALL {
                    let v = match a {
                        GamepadAxis::LeftTrigger => {
                            trigger(&pad, Button::LeftTrigger2, Axis::LeftZ)
                        }
                        GamepadAxis::RightTrigger => {
                            trigger(&pad, Button::RightTrigger2, Axis::RightZ)
                        }
                        _ => pad.value(axis(a)),
                    };
                    // Strongest pad wins.
                    if v.abs() > state.axis(a).abs() {
                        state.set_axis(a, v);
                    }
                }
            }
        }
    }

    fn trigger(pad: &gilrs::Gamepad<'_>, b: Button, a: Axis) -> f32 {
        let analog = pad.button_data(b).map_or(0.0, |d| d.value());
        analog.max(pad.value(a).max(0.0))
    }

    fn button(b: GamepadButton) -> Button {
        match b {
            GamepadButton::South => Button::South,
            GamepadButton::East => Button::East,
            GamepadButton::North => Button::North,
            GamepadButton::West => Button::West,
            GamepadButton::LeftBumper => Button::LeftTrigger,
            GamepadButton::RightBumper => Button::RightTrigger,
            GamepadButton::LeftTrigger => Button::LeftTrigger2,
            GamepadButton::RightTrigger => Button::RightTrigger2,
            GamepadButton::Select => Button::Select,
            GamepadButton::Start => Button::Start,
            GamepadButton::Mode => Button::Mode,
            GamepadButton::LeftStick => Button::LeftThumb,
            GamepadButton::RightStick => Button::RightThumb,
            GamepadButton::DPadUp => Button::DPadUp,
            GamepadButton::DPadDown => Button::DPadDown,
            GamepadButton::DPadLeft => Button::DPadLeft,
            GamepadButton::DPadRight => Button::DPadRight,
        }
    }

    fn axis(a: GamepadAxis) -> Axis {
        match a {
            GamepadAxis::LeftStickX => Axis::LeftStickX,
            GamepadAxis::LeftStickY => Axis::LeftStickY,
            GamepadAxis::RightStickX => Axis::RightStickX,
            GamepadAxis::RightStickY => Axis::RightStickY,
            GamepadAxis::LeftTrigger => Axis::LeftZ,
            GamepadAxis::RightTrigger => Axis::RightZ,
        }
    }
}
