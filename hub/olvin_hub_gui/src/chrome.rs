//! Window chrome: the launcher backdrop, sidebar, status line, toasts, the
//! page transition and the shared page header.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Painter, Rect, Sense, Stroke, Ui, UiBuilder,
    epaint::Mesh, pos2, vec2,
};
use olvin_hub::EngineSource;

use crate::ui::{self, FS_2XL, FS_MD, FS_SM, FS_XL, Palette, RADIUS_MD, anim, logo};
use crate::widgets::{self, Btn, Icon, bold, mono, paint_icon, sans, since, truncated, tween};
use crate::{Hub, Page};

/// `--bg-radial`: `radial-gradient(circle at 50% 0%, #2c2c2e 0%, #000 100%)`,
/// the launcher-only background (STYLE.md §1).
pub fn paint_backdrop(painter: &Painter, rect: Rect, p: &Palette) {
    painter.rect_filled(rect, 0.0, p.bg);
    let dark = p.dark;
    let top = if dark {
        anim::lerp_color(p.bg, p.text, 0.16)
    } else {
        p.surface
    };
    let center = pos2(rect.center().x, rect.min.y);
    // `circle` with the default `farthest-corner` size.
    let radius = (rect.width() / 2.0).hypot(rect.height());
    const RINGS: usize = 24;
    const SEGMENTS: usize = 48;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, top);
    for ring in 1..=RINGS {
        let t = ring as f32 / RINGS as f32;
        let color = anim::lerp_color(top, p.bg, t);
        for seg in 0..=SEGMENTS {
            let a = std::f32::consts::PI * seg as f32 / SEGMENTS as f32;
            mesh.colored_vertex(center + vec2(a.cos(), a.sin()) * radius * t, color);
        }
    }
    let row = (SEGMENTS + 1) as u32;
    for seg in 0..SEGMENTS as u32 {
        mesh.add_triangle(0, 1 + seg, 2 + seg);
    }
    for ring in 0..(RINGS - 1) as u32 {
        let a = 1 + ring * row;
        let b = a + row;
        for seg in 0..SEGMENTS as u32 {
            mesh.add_triangle(a + seg, b + seg, a + seg + 1);
            mesh.add_triangle(a + seg + 1, b + seg, b + seg + 1);
        }
    }
    painter.with_clip_rect(rect).add(mesh);
}

/// Page title with a subtitle and right-aligned actions. Narrow pages put
/// the actions on their own line under the title.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, actions: impl FnOnce(&mut Ui)) {
    let p = ui::palette(ui.ctx());
    let w = ui.available_width();
    let stacked = w < 560.0;
    let title_block = |ui: &mut Ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                egui::RichText::new(title)
                    .size(if stacked { FS_XL + 4.0 } else { FS_2XL })
                    .family(ui::bold())
                    .color(p.text),
            );
            if !subtitle.is_empty() {
                ui.add(
                    egui::Label::new(egui::RichText::new(subtitle).size(FS_MD).color(p.muted))
                        .truncate(),
                );
            }
        });
    };
    if stacked {
        title_block(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            actions(ui);
        });
    } else {
        ui.allocate_ui_with_layout(
            vec2(w, 56.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                title_block(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    actions(ui);
                });
            },
        );
    }
    ui.add_space(14.0);
}

/// `--fs-xs` caps section label.
pub fn section(ui: &mut Ui, text: &str) {
    let p = ui::palette(ui.ctx());
    ui.add_space(6.0);
    ui.label(ui::caps(text).color(p.muted));
    ui.add_space(4.0);
}

impl Hub {
    pub(crate) fn sidebar(&mut self, ui: &mut Ui, rail: bool, now: f64) {
        let ctx = ui.ctx().clone();
        let p = ui::palette(&ctx);
        if rail {
            ui.vertical_centered(|ui| logo::mark(ui, 34.0).on_hover_text("Olvin Hub"));
        } else {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 10.0;
                let intro = self.intro_at.unwrap_or(now);
                let r = logo::wordmark_intro(ui, 24.0, intro);
                // "Hub" sits on the wordmark's baseline.
                ui.painter().text(
                    pos2(r.rect.max.x + 4.0, r.rect.max.y + 1.0),
                    Align2::LEFT_BOTTOM,
                    "Hub",
                    sans(FS_XL - 1.0),
                    p.muted,
                );
            });
        }
        ui.add_space(26.0);

        let items = [
            (Page::Projects, Icon::Folder, "Projects"),
            (Page::Editor, Icon::Pencil, "Editor"),
            (Page::Settings, Icon::Gear, "Settings"),
        ];
        let item_h = 40.0;
        let gap = 4.0;
        let top = ui.cursor().min.y;
        let width = ui.available_width();
        let mut selected_y = top;
        let mut clicked = None;
        for (i, (page, icon, label)) in items.iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                pos2(ui.cursor().min.x, top + i as f32 * (item_h + gap)),
                vec2(width, item_h),
            );
            let response = ui
                .interact(rect, Id::new(("nav", i)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            let response = if rail {
                response.on_hover_text(*label)
            } else {
                response
            };
            if response.clicked() {
                clicked = Some(*page);
            }
            let on = self.page == *page;
            if on {
                selected_y = rect.min.y;
            }
            let hover = anim::hover_t(&ctx, response.id.with("h"), response.hovered());
            let sel = anim::fade(&ctx, response.id.with("sel"), on);
            let fill = anim::lerp_color(
                anim::lerp_color(Color32::TRANSPARENT, p.glass, hover),
                p.line,
                sel,
            );
            ui.painter().rect_filled(rect, RADIUS_MD, fill);
            let color = anim::lerp_color(anim::lerp_color(p.muted, p.text, hover), p.text, sel);
            let icon_rect = if rail {
                Rect::from_center_size(rect.center(), vec2(20.0, 20.0))
            } else {
                Rect::from_center_size(pos2(rect.min.x + 24.0, rect.center().y), vec2(18.0, 18.0))
            };
            paint_icon(
                ui.painter(),
                icon_rect,
                *icon,
                anim::lerp_color(color, p.accent, sel),
            );
            if !rail {
                ui.painter().text(
                    pos2(rect.min.x + 44.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    if on {
                        bold(FS_MD + 1.0)
                    } else {
                        sans(FS_MD + 1.0)
                    },
                    color,
                );
            }
        }
        // The selection bar slides between items.
        let y = tween(&ctx, Id::new("nav-indicator"), selected_y, 0.35);
        let bar = Rect::from_min_size(pos2(ui.cursor().min.x - 2.0, y + 11.0), vec2(3.0, 18.0));
        let glow = egui::Shadow {
            offset: [0, 0],
            blur: 10,
            spread: 0,
            color: p.accent_glow,
        };
        ui.painter().add(glow.as_shape(bar, 2));
        ui.painter().rect_filled(bar, 2.0, p.accent);
        ui.advance_cursor_after_rect(Rect::from_min_size(
            pos2(ui.cursor().min.x, top),
            vec2(width, items.len() as f32 * (item_h + gap)),
        ));
        if let Some(page) = clicked {
            self.set_page(page, now);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if !rail {
                ui.label(
                    egui::RichText::new(format!("Olvin Hub {}", env!("CARGO_PKG_VERSION")))
                        .size(FS_SM)
                        .color(p.faint),
                );
            }
        });
    }

    /// The footer: a status dot with what's going on, the engine source on the right.
    pub(crate) fn status_line(&mut self, ui: &mut Ui) {
        let p = ui::palette(ui.ctx());
        let rect = ui.max_rect();
        let painter = ui.painter().clone();
        painter.hline(rect.x_range(), rect.min.y, Stroke::new(1.0, p.line));
        let cy = rect.center().y;

        let (dot, text) = if self.build.is_some() {
            (p.warn, "Building editor".to_owned())
        } else if let Some((name, _)) = &self.game {
            (p.info, format!("{name} running"))
        } else {
            (p.accent, "Ready".to_owned())
        };
        widgets::status_dot(&painter, pos2(rect.min.x + 4.0, cy), dot);
        let left = painter.text(
            pos2(rect.min.x + 14.0, cy),
            Align2::LEFT_CENTER,
            &text,
            sans(FS_SM),
            p.text_2,
        );
        let mut left_end = left.max.x + 12.0;
        if self.game.is_some() {
            let r = Rect::from_min_size(pos2(left_end, cy - 11.0), vec2(64.0, 22.0));
            let stop = ui
                .put(r, Btn::secondary("Stop").icon(Icon::Stop).height(22.0))
                .on_hover_text("Stop the running game");
            if stop.clicked() {
                self.stop_game();
            }
            left_end = r.max.x + 12.0;
        }

        // Engine · path · /home/… (mono), truncated from the right.
        let (kind, value) = match &self.config.engine {
            EngineSource::Path(path) => ("path", path.display().to_string()),
            EngineSource::Git(url) => ("git", url.clone()),
        };
        let avail = rect.max.x - left_end;
        if avail < 80.0 {
            return;
        }
        let compact = avail < 320.0;
        let label = widgets::caps_galley(&painter, "Engine", p.muted);
        let kind_g = painter.layout_no_wrap(format!("{kind}  ·  "), mono(FS_SM - 1.0), p.muted);
        let prefix = if compact {
            0.0
        } else {
            label.size().x + 10.0 + kind_g.size().x
        };
        let value_g = truncated(
            &painter,
            &value,
            mono(FS_SM - 1.0),
            p.text_2,
            avail - prefix - 6.0,
        );
        let total = prefix + value_g.size().x;
        let mut x = rect.max.x - total;
        let hover_rect = Rect::from_min_max(pos2(x, rect.min.y), rect.max);
        if !compact {
            painter.galley(pos2(x, cy - label.size().y / 2.0), label, p.muted);
            x += prefix - kind_g.size().x;
            painter.galley(pos2(x, cy - kind_g.size().y / 2.0), kind_g.clone(), p.muted);
            x += kind_g.size().x;
        }
        painter.galley(pos2(x, cy - value_g.size().y / 2.0), value_g, p.text_2);
        ui.interact(hover_rect, Id::new("engine-source"), Sense::hover())
            .on_hover_text(format!(
                "Engine for new projects: {kind} {value}\nChange it with olvin-hub engine --path <dir> or --git <url>"
            ));
    }

    /// Bottom-right toasts: slide in from the right and fade (300 ms),
    /// fade out at the end. Click to dismiss.
    pub(crate) fn toasts(&mut self, ctx: &egui::Context) {
        let p = ui::palette(ctx);
        let now = ctx.input(|i| i.time);
        let still = ui::reduce_motion(ctx);
        let screen = ctx.content_rect();
        let width = (screen.width() - 32.0).min(380.0);
        let mut offset = 44.0;
        let mut dismissed = Vec::new();
        for toast in self.toasts.iter_mut().rev() {
            let born = *toast.born.get_or_insert(now);
            let ttl = if toast.error { 8.0 } else { 4.0 };
            let age = (now - born) as f32;
            if age >= ttl {
                dismissed.push(toast.id);
                continue;
            }
            let appear = since(ctx, born, 0.3, anim::ease_out);
            let leave = ((age - (ttl - 0.25)) / 0.25).clamp(0.0, 1.0);
            let alpha = appear * (1.0 - leave);
            let slide = if still { 0.0 } else { 28.0 * (1.0 - appear) };
            ctx.request_repaint_after_secs((ttl - 0.25 - age).max(0.016));
            if leave > 0.0 {
                ctx.request_repaint();
            }
            let id = Id::new(("toast", toast.id));
            let area = egui::Area::new(id)
                .order(egui::Order::Tooltip)
                .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0 + slide, -offset))
                .interactable(true)
                .show(ctx, |ui| {
                    ui.multiply_opacity(alpha);
                    ui.set_width(width);
                    let accent = if toast.error { p.error } else { p.accent };
                    let frame = egui::Frame::new()
                        .fill(p.surface_raised)
                        .stroke(Stroke::new(1.0, p.line))
                        .corner_radius(RADIUS_MD)
                        .shadow(egui::Shadow {
                            offset: [0, 10],
                            blur: 40,
                            spread: 0,
                            color: Color32::from_black_alpha(150),
                        })
                        .inner_margin(egui::Margin {
                            left: 14,
                            right: 12,
                            top: 10,
                            bottom: 10,
                        });
                    frame
                        .show(ui, |ui| {
                            ui.set_width(width - 26.0);
                            ui.horizontal_top(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                                if toast.error {
                                    // Errors carry an icon, not just a colour (STYLE.md §11).
                                    ui.painter().circle_stroke(
                                        r.center(),
                                        7.0,
                                        Stroke::new(1.5, accent),
                                    );
                                    ui.painter().text(
                                        r.center(),
                                        Align2::CENTER_CENTER,
                                        "!",
                                        bold(11.0),
                                        accent,
                                    );
                                } else {
                                    widgets::status_dot(ui.painter(), r.center(), accent);
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&toast.text).size(FS_MD).color(p.text),
                                    )
                                    .wrap(),
                                );
                            });
                        })
                        .response
                        .interact(Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand)
                })
                .inner;
            if area.clicked() {
                dismissed.push(toast.id);
            }
            offset += area.rect.height() + 8.0;
        }
        self.toasts.retain(|t| !dismissed.contains(&t.id));
    }

    /// The current page in a centred column, fading and sliding in when the
    /// page changes.
    pub(crate) fn page(&mut self, ui: &mut Ui, now: f64) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let narrow = full.width() < 640.0;
        let margin = if narrow { 16.0 } else { 32.0 };
        let w = (full.width() - 2.0 * margin).clamp(200.0, 1180.0);
        let x0 = full.center().x - w / 2.0;
        let t = since(&ctx, self.page_at, 0.3, anim::ease_out);
        let rect = Rect::from_min_max(
            pos2(x0, full.min.y + if narrow { 18.0 } else { 30.0 }),
            pos2(x0 + w, full.max.y),
        )
        .translate(vec2(0.0, 10.0 * (1.0 - t)));
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(full);
        child.multiply_opacity(t);
        match self.page {
            Page::Projects => self.projects_page(&mut child, now),
            Page::Editor => self.editor_page(&mut child, now),
            Page::Settings => self.settings_page(&mut child),
        }
    }

    /// Offers a rebuild when the engine changed after the editor was built.
    pub(crate) fn stale_banner(&mut self, ui: &mut Ui) {
        if !self.editor_stale() {
            return;
        }
        let p = ui::palette(ui.ctx());
        let mut rebuild = false;
        egui::Frame::new()
            .fill(p.warn.gamma_multiply(0.12))
            .stroke(Stroke::new(1.0, p.warn.gamma_multiply(0.5)))
            .corner_radius(RADIUS_MD)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let narrow = ui.available_width() < 520.0;
                let text = |ui: &mut Ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("The engine changed after the editor was built")
                                .family(ui::bold())
                                .color(p.text),
                        );
                        ui.label(
                            egui::RichText::new(
                                "Rebuild to get the new features. Opening a project does it for you.",
                            )
                            .color(p.text_2),
                        );
                    });
                };
                if narrow {
                    text(ui);
                    rebuild = ui.add(Btn::secondary("Rebuild").icon(Icon::Refresh)).clicked();
                } else {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        paint_icon(ui.painter(), r, Icon::Refresh, p.warn);
                        text(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            rebuild = ui
                                .add(Btn::secondary("Rebuild").icon(Icon::Refresh))
                                .clicked();
                        });
                    });
                }
            });
        ui.add_space(12.0);
        if rebuild {
            let wanted = self.editor_selection();
            self.start_build(&wanted);
        }
    }
}
