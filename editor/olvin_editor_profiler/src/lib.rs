//! Editor element: Profiler panel. Frame time history, the render counters
//! (draw calls, instances, culled, triangles) and per-scope timings (systems,
//! render passes, physics, scripts) from the engine's puffin scopes.
//!
//! puffin_egui targets another egui than the editor's, so the views are drawn
//! here from the frame data (`olvin::core::profile`).

use std::collections::{HashMap, VecDeque};

use olvin::core::profile::{self, Collector, FrameProfile};
use olvin::render::RenderStats;
use olvin_editor::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, vec2};
use olvin_editor::{EditorPlugin, EditorState, Panel, PanelKind, Registry};

/// Panel id in saved layouts.
pub const PROFILER: &str = "profiler.stats";

/// Frame times kept for the graph.
const HISTORY: usize = 240;
/// Rows of the scope table.
const TABLE_ROWS: usize = 40;

pub struct ProfilerPlugin;

impl EditorPlugin for ProfilerPlugin {
    fn name(&self) -> &'static str {
        "Profiler"
    }

    fn build(&self, registry: &mut Registry) {
        registry
            .add_panel(PanelKind {
                id: PROFILER,
                title: "⏱ Profiler",
                factory: || Box::new(Profiler::new()),
            })
            // One profiler frame per editor frame (a no-op while no panel records).
            .add_frame_hook(|_, _| profile::new_frame());
    }
}

struct Profiler {
    collector: Collector,
    /// Frame times in ms, oldest first.
    history: VecDeque<f32>,
    paused: bool,
    /// The frame shown in the timeline and table (the newest one unless paused).
    frame: Option<FrameProfile>,
}

impl Profiler {
    fn new() -> Self {
        Self {
            collector: Collector::new(),
            history: VecDeque::with_capacity(HISTORY),
            paused: false,
            frame: None,
        }
    }
}

impl Drop for Profiler {
    fn drop(&mut self) {
        // The panel was closed: stop recording scopes.
        profile::set_enabled(false);
    }
}

impl Panel for Profiler {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        profile::set_enabled(true);
        if !self.paused {
            let dt_ms = ui.input(|i| i.unstable_dt) * 1000.0;
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(dt_ms);
            if let Some(frame) = self.collector.latest() {
                self.frame = Some(frame);
            }
            // A profiler measures a running loop, not an idle one.
            ui.ctx().request_repaint();
        }

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                self.header(ui);
                self.graph(ui);
                ui.add_space(6.0);
                counters(ui, &totals(state));
                ui.add_space(6.0);
                self.timeline(ui);
                ui.add_space(6.0);
                self.table(ui);
            });
    }
}

/// The render counters of every view drawn last frame, added up.
fn totals(state: &EditorState) -> RenderStats {
    let mut sum = RenderStats::default();
    for s in state.render_stats.values() {
        sum.draw_calls += s.draw_calls;
        sum.instances += s.instances;
        sum.culled += s.culled;
        sum.triangles += s.triangles;
    }
    sum
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

impl Profiler {
    fn header(&mut self, ui: &mut egui::Ui) {
        let last = self.history.back().copied().unwrap_or(0.0);
        let avg = self.history.iter().sum::<f32>() / self.history.len().max(1) as f32;
        let max = self.history.iter().copied().fold(0.0, f32::max);
        ui.horizontal(|ui| {
            if ui
                .button(if self.paused {
                    "▶ Resume"
                } else {
                    "⏸ Pause"
                })
                .clicked()
            {
                self.paused = !self.paused;
            }
            ui.label(
                RichText::new(format!(
                    "Frame {last:.2} ms ({:.0} fps) · avg {avg:.2} ms · max {max:.2} ms",
                    1000.0 / last.max(1e-3)
                ))
                .monospace(),
            );
        });
    }

    /// Frame time bars with 60 and 30 fps guides.
    fn graph(&self, ui: &mut egui::Ui) {
        let height = 80.0;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);
        let top = self.history.iter().copied().fold(33.4, f32::max).min(200.0);
        let y_of = |ms: f32| rect.bottom() - (ms / top).min(1.0) * (height - 4.0) - 2.0;
        for (ms, label) in [(16.67, "60 fps"), (33.33, "30 fps")] {
            if ms < top {
                let y = y_of(ms);
                painter.hline(
                    rect.x_range(),
                    y,
                    Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.4)),
                );
                painter.text(
                    Pos2::new(rect.right() - 4.0, y),
                    egui::Align2::RIGHT_BOTTOM,
                    label,
                    egui::FontId::monospace(10.0),
                    visuals.weak_text_color(),
                );
            }
        }
        let bar = rect.width() / HISTORY as f32;
        let offset = HISTORY - self.history.len();
        for (i, &ms) in self.history.iter().enumerate() {
            let x = rect.left() + (offset + i) as f32 * bar;
            let color = if ms <= 16.7 {
                Color32::from_rgb(90, 190, 120)
            } else if ms <= 33.4 {
                Color32::from_rgb(230, 190, 70)
            } else {
                Color32::from_rgb(225, 90, 80)
            };
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(x, y_of(ms)),
                    Pos2::new(x + bar.max(1.0), rect.bottom()),
                ),
                0.0,
                color,
            );
        }
        response.on_hover_text("Frame time, newest on the right");
    }

    /// Scopes of the shown frame as bars by depth, on the first thread.
    fn timeline(&self, ui: &mut egui::Ui) {
        ui.label(olvin_editor::theme::caps(ui.ctx(), "Timeline"));
        let Some(frame) = &self.frame else {
            ui.label(RichText::new("Waiting for a frame…").weak());
            return;
        };
        let total = frame.duration_ns.max(1) as f32;
        let row = 16.0;
        let depth = frame.scopes.iter().map(|s| s.depth).max().unwrap_or(0) + 1;
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), row * depth.min(12) as f32),
            Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
        for s in frame.scopes.iter().filter(|s| s.thread == 0) {
            if s.depth >= 12 {
                continue;
            }
            let x0 = rect.left() + s.start_ns as f32 / total * rect.width();
            let w = (s.duration_ns as f32 / total * rect.width()).max(1.0);
            let r = Rect::from_min_size(
                Pos2::new(x0, rect.top() + s.depth as f32 * row),
                vec2(w, row - 1.0),
            );
            let hue = (hash(&s.name) % 360) as f32 / 360.0;
            let fill = egui::ecolor::Hsva::new(hue, 0.45, 0.65, 1.0);
            painter.rect_filled(r, 2.0, Color32::from(fill));
            painter.rect_stroke(r, 2.0, Stroke::new(0.5, Color32::BLACK), StrokeKind::Inside);
            if w > 40.0 {
                painter.with_clip_rect(r).text(
                    r.left_center() + vec2(3.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &s.name,
                    egui::FontId::monospace(10.0),
                    Color32::BLACK,
                );
            }
        }
        if let Some(pos) = response.hover_pos() {
            let depth_at = ((pos.y - rect.top()) / row) as u32;
            let t = ((pos.x - rect.left()) / rect.width() * total) as i64;
            if let Some(s) = frame.scopes.iter().find(|s| {
                s.thread == 0
                    && s.depth == depth_at
                    && s.start_ns <= t
                    && t <= s.start_ns + s.duration_ns
            }) {
                response.on_hover_text(format!(
                    "{} · {:.3} ms",
                    s.name,
                    s.duration_ns as f32 / 1e6
                ));
            }
        }
    }

    /// Scopes grouped by name: systems, render passes, physics, scripts.
    fn table(&self, ui: &mut egui::Ui) {
        ui.label(olvin_editor::theme::caps(ui.ctx(), "Systems and passes"));
        let Some(frame) = &self.frame else {
            return;
        };
        // name -> (calls, total ns); nested scopes are counted on their own.
        let mut by_name: HashMap<&str, (u32, i64)> = HashMap::new();
        for s in &frame.scopes {
            let e = by_name.entry(&s.name).or_default();
            e.0 += 1;
            e.1 += s.duration_ns;
        }
        let mut rows: Vec<_> = by_name.into_iter().collect();
        rows.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(b.0)));
        let max = rows.first().map_or(1, |r| r.1.1.max(1)) as f32;
        egui::Grid::new("profiler.table")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Scope").weak());
                ui.label(RichText::new("ms").weak());
                ui.label(RichText::new("Calls").weak());
                ui.end_row();
                for (name, (calls, ns)) in rows.into_iter().take(TABLE_ROWS) {
                    let ms = ns as f32 / 1e6;
                    ui.label(RichText::new(name).monospace());
                    let (rect, _) = ui.allocate_exact_size(vec2(160.0, 14.0), Sense::hover());
                    let fill = Rect::from_min_size(
                        rect.min,
                        vec2(rect.width() * ns as f32 / max, rect.height()),
                    );
                    ui.painter()
                        .rect_filled(fill, 2.0, ui.visuals().selection.bg_fill);
                    ui.painter().text(
                        rect.left_center() + vec2(4.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{ms:.3}"),
                        egui::FontId::monospace(11.0),
                        ui.visuals().text_color(),
                    );
                    ui.label(RichText::new(calls.to_string()).monospace());
                    ui.end_row();
                }
            });
    }
}

fn counters(ui: &mut egui::Ui, stats: &RenderStats) {
    ui.label(olvin_editor::theme::caps(ui.ctx(), "Render"));
    egui::Grid::new("profiler.counters")
        .num_columns(2)
        .show(ui, |ui| {
            for (label, value) in [
                ("Draw calls", u64::from(stats.draw_calls)),
                ("Instances", u64::from(stats.instances)),
                ("Culled", u64::from(stats.culled)),
                ("Triangles", stats.triangles),
            ] {
                ui.label(label);
                ui.label(RichText::new(thousands(value)).monospace());
                ui.end_row();
            }
        });
}

/// FNV-1a: a stable colour per scope name.
fn hash(s: &str) -> u32 {
    s.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1 234 567");
    }
}
