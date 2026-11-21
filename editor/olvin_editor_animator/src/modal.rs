//! Blender-style modal key operations (G grab, S scale) for the Dope Sheet
//! and the Graph Editor. The operation is recomputed from a snapshot of the
//! action every frame, so confirming keeps the result and cancelling puts the
//! snapshot back.

use olvin_editor::egui::{self, Event, Key};
use olvin_editor::olvin_scene::action::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Grab,
    Scale,
}

/// Which components a graph grab moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Constraint {
    #[default]
    Free,
    /// Frames only.
    X,
    /// Values only.
    Y,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modal {
    pub kind: Kind,
    pub snapshot: Action,
    /// Mouse position (frame, value) when the operation started.
    pub start: (f32, f32),
    /// Typed number, as Blender's numeric input.
    pub typed: String,
    /// Snap frames to whole numbers (default on; Ctrl inverts).
    pub snap: bool,
    /// Started by dragging a key: releasing the mouse confirms.
    pub from_drag: bool,
    pub constraint: Constraint,
    /// The graph editor moves values too.
    pub graph: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Running,
    Confirm,
    Cancel,
}

impl Modal {
    pub fn new(
        kind: Kind,
        snapshot: Action,
        start: (f32, f32),
        from_drag: bool,
        graph: bool,
    ) -> Self {
        Self {
            kind,
            snapshot,
            start,
            typed: String::new(),
            snap: true,
            from_drag,
            constraint: Constraint::Free,
            graph,
        }
    }

    pub fn typed_value(&self) -> Option<f32> {
        self.typed.parse::<f32>().ok()
    }

    /// The action with the operation applied for the mouse at `(frame, value)`.
    pub fn result(&self, mouse: (f32, f32), current: f32, ctrl: bool) -> Action {
        let mut a = self.snapshot.clone();
        let snap = self.snap != ctrl;
        match self.kind {
            Kind::Grab => {
                let typed = self.typed_value();
                let mut df = mouse.0 - self.start.0;
                let mut dv = mouse.1 - self.start.1;
                match (self.constraint, typed) {
                    (Constraint::Y, Some(t)) => {
                        df = 0.0;
                        dv = t;
                    }
                    (Constraint::Y, None) => df = 0.0,
                    (Constraint::X, Some(t)) => {
                        df = t;
                        dv = 0.0;
                    }
                    (Constraint::X, None) => dv = 0.0,
                    (Constraint::Free, Some(t)) => {
                        df = t;
                        dv = 0.0;
                    }
                    (Constraint::Free, None) => {}
                }
                if !self.graph {
                    dv = 0.0;
                }
                if typed.is_some() || snap {
                    df = if snap { df.round() } else { df };
                }
                if self.graph {
                    a.translate_selected(df, dv);
                } else {
                    a.move_selected(df);
                }
                if snap && self.constraint != Constraint::Y {
                    a.snap_selected();
                }
            }
            Kind::Scale => {
                let base = self.start.0 - current;
                let factor = self.typed_value().unwrap_or(if base.abs() < 1e-3 {
                    1.0
                } else {
                    (mouse.0 - current) / base
                });
                a.scale_selected(current, factor, snap);
            }
        }
        a
    }

    /// Reads the keyboard and mouse for this frame: typed numbers, axis keys,
    /// confirm / cancel.
    pub fn input(&mut self, ctx: &egui::Context, mouse_released: bool) -> Outcome {
        let mut outcome = Outcome::Running;
        ctx.input(|i| {
            for e in &i.events {
                match e {
                    Event::Text(t) => {
                        for c in t.chars() {
                            if c.is_ascii_digit() || c == '.' || c == '-' {
                                self.typed.push(c);
                            } else if self.graph && (c == 'x' || c == 'X') {
                                self.constraint = toggle(self.constraint, Constraint::X);
                            } else if self.graph && (c == 'y' || c == 'Y') {
                                self.constraint = toggle(self.constraint, Constraint::Y);
                            }
                        }
                    }
                    Event::Key {
                        key: Key::Backspace,
                        pressed: true,
                        ..
                    } => {
                        self.typed.pop();
                    }
                    _ => {}
                }
            }
            if i.key_pressed(Key::Enter)
                || i.key_pressed(Key::Space)
                || (!self.from_drag && i.pointer.primary_pressed())
                || (self.from_drag && mouse_released)
            {
                outcome = Outcome::Confirm;
            }
            if i.key_pressed(Key::Escape) || i.pointer.secondary_pressed() {
                outcome = Outcome::Cancel;
            }
        });
        outcome
    }

    /// The status line, e.g. `Grab: 5.0 frames (X)`.
    pub fn status(&self) -> String {
        let name = match self.kind {
            Kind::Grab => "Grab",
            Kind::Scale => "Scale",
        };
        let typed = if self.typed.is_empty() {
            String::new()
        } else {
            format!(" [{}]", self.typed)
        };
        let axis = match self.constraint {
            Constraint::Free => "",
            Constraint::X => " X",
            Constraint::Y => " Y",
        };
        format!("{name}{typed}{axis}   Enter/LMB confirm   Esc/RMB cancel   Ctrl snap")
    }
}

fn toggle(current: Constraint, to: Constraint) -> Constraint {
    if current == to { Constraint::Free } else { to }
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::olvin_scene::action::{Prop, SEL_ALL};

    fn action() -> Action {
        let mut a = Action::default();
        for f in [1.0, 11.0] {
            a.insert_key(1, Prop::Location, 0, f, f);
        }
        a.curves[0].keys[1].select = SEL_ALL;
        a
    }

    #[test]
    fn grab_snaps_to_whole_frames_and_is_recomputed_from_the_snapshot() {
        let m = Modal::new(Kind::Grab, action(), (11.0, 0.0), false, false);
        let r = m.result((14.4, 0.0), 1.0, false);
        assert_eq!(r.curves[0].keys[1].frame, 14.0);
        // Ctrl inverts snapping.
        let r = m.result((14.4, 0.0), 1.0, true);
        assert!((r.curves[0].keys[1].frame - 14.4).abs() < 1e-4);
        // The snapshot is untouched.
        assert_eq!(m.snapshot.curves[0].keys[1].frame, 11.0);
    }

    #[test]
    fn typed_numbers_override_the_mouse() {
        let mut m = Modal::new(Kind::Grab, action(), (11.0, 0.0), false, false);
        m.typed = "-4".into();
        let r = m.result((100.0, 0.0), 1.0, false);
        assert_eq!(r.curves[0].keys[1].frame, 7.0);
        let mut s = Modal::new(Kind::Scale, action(), (21.0, 0.0), false, false);
        s.typed = "2".into();
        let r = s.result((0.0, 0.0), 1.0, false);
        assert_eq!(
            r.curves[0].keys[1].frame, 21.0,
            "11 scaled x2 around frame 1"
        );
    }

    #[test]
    fn scale_follows_the_mouse_around_the_current_frame() {
        let m = Modal::new(Kind::Scale, action(), (11.0, 0.0), false, false);
        // Mouse twice as far from the playhead (frame 1): keys spread x2.
        let r = m.result((21.0, 0.0), 1.0, false);
        assert_eq!(r.curves[0].keys[1].frame, 21.0);
        assert_eq!(r.curves[0].keys[0].frame, 1.0);
    }

    #[test]
    fn graph_grab_moves_values_and_honours_constraints() {
        let mut m = Modal::new(Kind::Grab, action(), (11.0, 11.0), false, true);
        let r = m.result((13.0, 20.0), 1.0, false);
        assert_eq!(
            (r.curves[0].keys[1].frame, r.curves[0].keys[1].value),
            (13.0, 20.0)
        );
        m.constraint = Constraint::Y;
        let r = m.result((13.0, 20.0), 1.0, false);
        assert_eq!(
            (r.curves[0].keys[1].frame, r.curves[0].keys[1].value),
            (11.0, 20.0)
        );
        m.constraint = Constraint::X;
        let r = m.result((13.0, 20.0), 1.0, false);
        assert_eq!(
            (r.curves[0].keys[1].frame, r.curves[0].keys[1].value),
            (13.0, 11.0)
        );
    }
}
