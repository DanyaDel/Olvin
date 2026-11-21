//! The animation clock: playback, the frame keys that work in every editor
//! (Blender's "Frames" keymap) and the per-frame evaluation of the animation
//! into the scene objects. State shared by the Timeline, Dope Sheet and
//! Graph Editor lives in a thread-local [`Shared`].

use std::cell::RefCell;

use olvin_editor::EditorState;
use olvin_editor::egui::{self, Key, Modifiers};
use olvin_editor::olvin_scene::action::{Action, KeyClipboard};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Forward,
    Reverse,
}

/// Blender's playback: frames advance at the scene's fps inside the range
/// and wrap around; Esc stops and returns to the frame where playback began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Playback {
    pub dir: Option<Dir>,
    /// The frame playback started from (Esc restores it).
    pub origin: i32,
    accum: f32,
}

impl Playback {
    pub fn is_playing(&self) -> bool {
        self.dir.is_some()
    }

    /// Space / Shift+Ctrl+Space: start in `dir`, or pause if playing.
    pub fn toggle(&mut self, dir: Dir, frame: i32) {
        if self.dir.is_some() {
            self.dir = None;
        } else {
            *self = Self {
                dir: Some(dir),
                origin: frame,
                accum: 0.0,
            };
        }
    }

    /// Esc during playback: stop and restore the starting frame.
    pub fn cancel(&mut self, frame: &mut i32) -> bool {
        if self.dir.take().is_some() {
            *frame = self.origin;
            true
        } else {
            false
        }
    }

    /// Advances `frame` by the time `dt` at `fps`, looping in `start..=end`.
    pub fn advance(&mut self, dt: f32, fps: f32, start: i32, end: i32, frame: &mut i32) {
        let Some(dir) = self.dir else {
            return;
        };
        let len = end - start + 1;
        if len <= 0 {
            self.dir = None;
            return;
        }
        if *frame < start || *frame > end {
            *frame = if dir == Dir::Forward { start } else { end };
            return;
        }
        self.accum += dt.max(0.0) * fps.max(1.0);
        let whole = self.accum.floor();
        self.accum -= whole;
        let n = whole as i32 % len;
        let delta = if dir == Dir::Forward { n } else { -n };
        *frame = start + (*frame - start + delta).rem_euclid(len);
    }
}

/// What the three editors share.
#[derive(Default)]
pub struct Shared {
    pub play: Playback,
    pub clipboard: KeyClipboard,
    applied: Option<Applied>,
}

struct Applied {
    frame: i32,
    generation: u64,
    action: Action,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared::default());
}

/// Runs `f` on the shared state.
pub fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    SHARED.with_borrow_mut(f)
}

/// The frame hook: global frame keys, playback, evaluation.
pub fn frame_hook(ctx: &egui::Context, state: &mut EditorState) {
    with(|shared| shared.update(ctx, state));
}

impl Shared {
    fn update(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        if !ctx.egui_wants_keyboard_input() && !state.busy {
            self.frame_keys(ctx, state);
        }
        if self.play.is_playing() {
            let a = &state.scene.animation;
            let dt = ctx.input(|i| i.stable_dt);
            let (fps, start, end) = (a.fps, a.frame_start, a.frame_end);
            self.play
                .advance(dt, fps, start, end, &mut state.anim_frame);
            ctx.request_repaint();
        }
        self.evaluate(state);
    }

    fn frame_keys(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        let (shift, ctrl, none) = (
            Modifiers::SHIFT,
            Modifiers::CTRL | Modifiers::SHIFT,
            Modifiers::NONE,
        );
        let pressed = |key: Key, mods: Modifiers| {
            ctx.input(|i| i.key_pressed(key) && i.modifiers.matches_exact(mods))
        };
        let a = &state.scene.animation;
        let (start, end) = (a.frame_start, a.frame_end);
        if pressed(Key::Space, none) {
            self.play.toggle(Dir::Forward, state.anim_frame);
        } else if pressed(Key::Space, ctrl) {
            self.play.toggle(Dir::Reverse, state.anim_frame);
        }
        if pressed(Key::Escape, none) {
            self.play.cancel(&mut state.anim_frame);
        }
        let mut jump = None;
        if pressed(Key::ArrowLeft, none) {
            state.anim_frame -= 1;
            self.play.dir = None;
        } else if pressed(Key::ArrowRight, none) {
            state.anim_frame += 1;
            self.play.dir = None;
        } else if pressed(Key::ArrowLeft, shift) {
            state.anim_frame = start;
            self.play.dir = None;
        } else if pressed(Key::ArrowRight, shift) {
            state.anim_frame = end;
            self.play.dir = None;
        } else if pressed(Key::ArrowUp, none) {
            jump = Some(true);
        } else if pressed(Key::ArrowDown, none) {
            jump = Some(false);
        }
        if let Some(forward) = jump {
            jump_keyframe(state, forward);
        }
    }

    /// Writes the animation at the current frame into the scene when the
    /// frame, the animation data or the scene itself (undo, open) changed.
    /// A hand-edited property stays as edited until one of those happens.
    fn evaluate(&mut self, state: &mut EditorState) {
        let stale = self.applied.as_ref().is_none_or(|a| {
            a.frame != state.anim_frame
                || a.generation != state.scene_generation
                || !a.action.same_data(&state.scene.animation)
        });
        if stale {
            state.apply_animation_quietly(state.anim_frame as f32);
            self.applied = Some(Applied {
                frame: state.anim_frame,
                generation: state.scene_generation,
                action: state.scene.animation.clone(),
            });
        }
    }
}

/// Up / Down: the next or previous key frame of the selected objects.
pub fn jump_keyframe(state: &mut EditorState, forward: bool) {
    let selected = state.selected_ids();
    let target =
        state
            .scene
            .animation
            .jump_frame(state.anim_frame as f32, forward, Some(&selected));
    match target {
        Some(f) => state.anim_frame = f.round() as i32,
        None => state.info("no more keyframes in that direction"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_loops_in_the_range_at_the_fps() {
        let mut p = Playback::default();
        let mut frame = 249;
        p.toggle(Dir::Forward, frame);
        // 1/24 s per frame: a half-frame step does not move yet.
        p.advance(1.0 / 48.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 249);
        p.advance(1.0 / 48.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 250);
        p.advance(1.0 / 24.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 1, "wraps to the start");
        // A slow frame skips frames instead of slowing down.
        p.advance(5.0 / 24.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 6);
    }

    #[test]
    fn reverse_wraps_to_the_end_and_esc_restores_the_start_frame() {
        let mut p = Playback::default();
        let mut frame = 2;
        p.toggle(Dir::Reverse, frame);
        p.advance(2.0 / 24.0, 24.0, 1, 10, &mut frame);
        assert_eq!(frame, 10);
        assert!(p.cancel(&mut frame));
        assert_eq!(frame, 2);
        assert!(!p.is_playing());
        assert!(!p.cancel(&mut frame), "nothing to cancel when stopped");
    }

    #[test]
    fn space_pauses_in_place() {
        let mut p = Playback::default();
        let mut frame = 5;
        p.toggle(Dir::Forward, frame);
        p.advance(3.0 / 24.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 8);
        p.toggle(Dir::Forward, frame);
        assert!(!p.is_playing());
        p.advance(1.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 8);
    }

    #[test]
    fn playing_from_outside_the_range_enters_it() {
        let mut p = Playback::default();
        let mut frame = 400;
        p.toggle(Dir::Forward, frame);
        p.advance(0.0, 24.0, 1, 250, &mut frame);
        assert_eq!(frame, 1);
    }
}
