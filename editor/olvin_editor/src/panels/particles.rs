//! The Particle System component of the inspector: Unity-style groups
//! (Main, Emission, Start values, Shape, Forces, Colour / Size over Lifetime,
//! Renderer), a Presets menu and the preview buttons of the Scene view.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use olvin_scene::file::{BlendDesc, BurstDesc, ParticleDesc, ShapeDesc, SpaceDesc};

use super::inspector::{drag, grid, row};
use crate::fx::FxPreview;
use crate::{theme, widgets};

/// The preview buttons and live counters of one object, from `FxPreview`.
pub(crate) struct Preview<'a> {
    pub fx: &'a mut FxPreview,
    pub id: u64,
}

/// `[min, max]` as two drag fields; min never passes max.
pub(crate) fn range(ui: &mut Ui, r: &mut [f32; 2], speed: f64, suffix: &str) {
    let w = ((super::inspector::cell_width(ui) - 24.0) / 2.0).clamp(36.0, 90.0);
    let field = |ui: &mut Ui, v: &mut f32| {
        ui.add_sized(
            [w, 18.0],
            egui::DragValue::new(v)
                .speed(speed)
                .max_decimals(3)
                .suffix(suffix),
        )
        .changed()
    };
    let (a, b) = (field(ui, &mut r[0]), field(ui, &mut r[1]));
    if a && r[0] > r[1] {
        r[1] = r[0];
    }
    if b && r[1] < r[0] {
        r[0] = r[1];
    }
}

fn group(ui: &mut Ui, title: &str, open: bool, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(("particle-group", title))
        .default_open(open)
        .show(ui, body);
}

/// The whole block (inside `section`).
pub(crate) fn particle_ui(ui: &mut Ui, d: &mut ParticleDesc, preview: Option<Preview>) {
    let pal = theme::palette(ui.ctx());
    if let Some(Preview { fx, id }) = preview {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("▶ Play")
                .on_hover_text("Play or resume")
                .clicked()
            {
                fx.play(id);
            }
            if ui.button("⏸ Pause").clicked() {
                fx.pause(id);
            }
            if ui.button("⟲ Restart").clicked() {
                fx.restart(id);
            }
            if ui
                .button("⏹ Stop")
                .on_hover_text("Clear and stop")
                .clicked()
            {
                fx.stop(id);
            }
        });
        ui.horizontal(|ui| {
            let c = fx.control(id);
            ui.checkbox(&mut c.simulate, "Simulate")
                .on_hover_text("Keep simulating in the Scene view when not selected");
            let status = if fx.on_gpu(id) {
                "GPU simulation".to_owned()
            } else if let Some(n) = fx.count(id) {
                format!("{n} particles")
            } else {
                "not simulating".to_owned()
            };
            ui.colored_label(pal.muted, status);
        });
    }
    ui.horizontal(|ui| {
        ui.label("Presets");
        egui::ComboBox::from_id_salt("particle-presets")
            .selected_text("Apply…")
            .show_ui(ui, |ui| {
                for (name, make) in ParticleDesc::PRESETS {
                    if ui.selectable_label(false, *name).clicked() {
                        // The preset keeps what is about the project, not the look.
                        let (gpu, texture) = (d.gpu, d.texture.take());
                        *d = make();
                        d.gpu = gpu;
                        d.texture = texture;
                    }
                }
            });
    });
    group(ui, "Main", true, |ui| {
        grid(ui, "p-main", |ui| {
            row(ui, "Duration", |ui| {
                drag(ui, &mut d.duration, 0.05..=1000.0, 0.05, " s")
            });
            row(ui, "Looping", |ui| {
                ui.checkbox(&mut d.looping, "");
            });
            row(ui, "Prewarm", |ui| {
                ui.checkbox(&mut d.prewarm, "");
            });
            row(ui, "Play On Awake", |ui| {
                ui.checkbox(&mut d.play_on_awake, "");
            });
            row(ui, "Max Particles", |ui| {
                ui.add(
                    egui::DragValue::new(&mut d.max_particles)
                        .range(1..=1_048_576)
                        .speed(10.0),
                );
            });
            row(ui, "Space", |ui| {
                egui::ComboBox::from_id_salt("p-space")
                    .selected_text(match d.space {
                        SpaceDesc::World => "World",
                        SpaceDesc::Local => "Local",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.space, SpaceDesc::World, "World");
                        ui.selectable_value(&mut d.space, SpaceDesc::Local, "Local");
                    });
            });
            row(ui, "GPU", |ui| {
                ui.checkbox(&mut d.gpu, "")
                    .on_hover_text("Compute-shader simulation on the Full tier; the CPU elsewhere");
                if d.gpu && !olvin::render::gpu_particles_available() {
                    ui.colored_label(pal.warn, "no compute here: CPU");
                }
            });
            row(ui, "Seed", |ui| {
                ui.add(egui::DragValue::new(&mut d.seed));
            });
        });
    });
    group(ui, "Emission", true, |ui| {
        grid(ui, "p-emission", |ui| {
            row(ui, "Rate over Time", |ui| {
                drag(ui, &mut d.rate, 0.0..=1_000_000.0, 0.5, "/s")
            });
        });
        let mut remove = None;
        for (i, b) in d.bursts.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("Burst");
                drag(ui, &mut b.time, 0.0..=1000.0, 0.01, " s");
                ui.add(egui::DragValue::new(&mut b.count).speed(1.0).suffix(" ×"));
                if ui.small_button("✖").on_hover_text("Remove burst").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            d.bursts.remove(i);
        }
        if ui.small_button("+ Add burst").clicked() {
            d.bursts.push(BurstDesc {
                time: 0.0,
                count: 30,
            });
        }
    });
    group(ui, "Start values", true, |ui| {
        grid(ui, "p-start", |ui| {
            row(ui, "Lifetime", |ui| range(ui, &mut d.lifetime, 0.02, " s"));
            row(ui, "Speed", |ui| range(ui, &mut d.speed, 0.05, ""));
            row(ui, "Size", |ui| range(ui, &mut d.size, 0.01, ""));
            row(ui, "Rotation", |ui| range(ui, &mut d.rotation, 1.0, "°"));
            row(ui, "Spin", |ui| range(ui, &mut d.spin, 1.0, "°/s"));
            row(ui, "Color", |ui| {
                widgets::color_rgba(ui, &mut d.color);
                let mut random = d.color_max.is_some();
                if ui
                    .checkbox(&mut random, "Random")
                    .on_hover_text("Each particle picks a colour between the two")
                    .changed()
                {
                    d.color_max = random.then_some(d.color);
                }
                if let Some(max) = &mut d.color_max {
                    widgets::color_rgba(ui, max);
                }
            });
        });
    });
    group(ui, "Shape", false, |ui| shape_ui(ui, &mut d.shape));
    group(ui, "Forces", false, |ui| {
        grid(ui, "p-forces", |ui| {
            row(ui, "Gravity", |ui| {
                for v in d.gravity.as_mut() {
                    ui.add(egui::DragValue::new(v).speed(0.05).max_decimals(2));
                }
            });
            row(ui, "Drag", |ui| drag(ui, &mut d.drag, 0.0..=50.0, 0.01, ""));
        });
    });
    group(ui, "Color over Lifetime", false, |ui| {
        gradient_ui(ui, &mut d.color_over_life)
    });
    group(ui, "Size over Lifetime", false, |ui| {
        curve_ui(ui, &mut d.size_over_life)
    });
    group(ui, "Renderer", false, |ui| {
        grid(ui, "p-render", |ui| {
            row(ui, "Texture", |ui| {
                let mut path = d.texture.clone().unwrap_or_default();
                let hint = egui::TextEdit::singleline(&mut path)
                    .hint_text("soft circle")
                    .desired_width(super::inspector::cell_width(ui).clamp(60.0, 200.0));
                if ui.add(hint).changed() {
                    d.texture = (!path.trim().is_empty()).then(|| path.trim().to_owned());
                }
            });
            row(ui, "Blend", |ui| {
                egui::ComboBox::from_id_salt("p-blend")
                    .selected_text(match d.blend {
                        BlendDesc::Alpha => "Alpha",
                        BlendDesc::Additive => "Additive",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.blend, BlendDesc::Alpha, "Alpha");
                        ui.selectable_value(&mut d.blend, BlendDesc::Additive, "Additive");
                    });
            });
            row(ui, "Flip-book", |ui| {
                let mut on = d.flip_book.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    d.flip_book = on.then_some((4, 4));
                }
                if let Some((c, r)) = &mut d.flip_book {
                    ui.add(egui::DragValue::new(c).range(1..=64).suffix(" cols"));
                    ui.add(egui::DragValue::new(r).range(1..=64).suffix(" rows"));
                }
            });
            row(ui, "Sort", |ui| {
                ui.checkbox(&mut d.sort, "")
                    .on_hover_text("Far to near (alpha, CPU)");
            });
        });
    });
}

fn shape_ui(ui: &mut Ui, shape: &mut ShapeDesc) {
    let label = |s: &ShapeDesc| match s {
        ShapeDesc::Point => "Point",
        ShapeDesc::Sphere { .. } => "Sphere",
        ShapeDesc::Hemisphere { .. } => "Hemisphere",
        ShapeDesc::Cone { .. } => "Cone",
        ShapeDesc::Box { .. } => "Box",
        ShapeDesc::Circle { .. } => "Circle",
    };
    let choices = [
        ShapeDesc::Point,
        ShapeDesc::Sphere {
            radius: 1.0,
            surface: false,
        },
        ShapeDesc::Hemisphere {
            radius: 1.0,
            surface: false,
        },
        ShapeDesc::Cone {
            angle_deg: 25.0,
            radius: 0.1,
        },
        ShapeDesc::Box {
            half_extents: glam::Vec3::splat(0.5),
        },
        ShapeDesc::Circle {
            radius: 1.0,
            edge: false,
        },
    ];
    grid(ui, "p-shape", |ui| {
        row(ui, "Shape", |ui| {
            egui::ComboBox::from_id_salt("p-shape-kind")
                .selected_text(label(shape))
                .show_ui(ui, |ui| {
                    for c in choices {
                        let same = label(&c) == label(shape);
                        if ui.selectable_label(same, label(&c)).clicked() && !same {
                            *shape = c;
                        }
                    }
                });
        });
        match shape {
            ShapeDesc::Point => {}
            ShapeDesc::Sphere { radius, surface } | ShapeDesc::Hemisphere { radius, surface } => {
                row(ui, "Radius", |ui| drag(ui, radius, 0.0..=1000.0, 0.01, ""));
                row(ui, "Surface only", |ui| {
                    ui.checkbox(surface, "");
                });
            }
            ShapeDesc::Cone { angle_deg, radius } => {
                row(ui, "Angle", |ui| drag(ui, angle_deg, 0.0..=89.0, 0.5, "°"));
                row(ui, "Radius", |ui| drag(ui, radius, 0.0..=1000.0, 0.01, ""));
            }
            ShapeDesc::Box { half_extents } => row(ui, "Half size", |ui| {
                for v in half_extents.as_mut() {
                    ui.add(
                        egui::DragValue::new(v)
                            .speed(0.02)
                            .range(0.0..=1000.0)
                            .max_decimals(2),
                    );
                }
            }),
            ShapeDesc::Circle { radius, edge } => {
                row(ui, "Radius", |ui| drag(ui, radius, 0.0..=1000.0, 0.01, ""));
                row(ui, "Edge only", |ui| {
                    ui.checkbox(edge, "");
                });
            }
        }
    });
}

fn to_color(c: [f32; 4]) -> Color32 {
    // Linear RGBA to display colour, for the preview bar only.
    let enc = |v: f32| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
    Color32::from_rgba_unmultiplied(
        enc(c[0]),
        enc(c[1]),
        enc(c[2]),
        (c[3].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Colour stops: a preview bar and one row per stop.
fn gradient_ui(ui: &mut Ui, stops: &mut Vec<(f32, [f32; 4])>) {
    let pal = theme::palette(ui.ctx());
    let width = super::inspector::cell_width(ui).clamp(80.0, 400.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, pal.surface);
    let n = 48;
    let at = |t: f32| {
        let i = stops.partition_point(|(s, _)| *s <= t);
        match (i.checked_sub(1).map(|i| stops[i]), stops.get(i).copied()) {
            (None, None) => [1.0; 4],
            (Some((_, c)), None) | (None, Some((_, c))) => c,
            (Some((t0, a)), Some((t1, b))) => {
                let f = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
                std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f)
            }
        }
    };
    for i in 0..n {
        let (x0, x1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        let strip = Rect::from_min_max(
            Pos2::new(rect.left() + x0 * width, rect.top()),
            Pos2::new(rect.left() + x1 * width + 0.5, rect.bottom()),
        );
        painter.rect_filled(strip, 0.0, to_color(at((x0 + x1) * 0.5)));
    }
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, pal.line),
        egui::StrokeKind::Inside,
    );
    let mut remove = None;
    for (i, (t, c)) in stops.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            drag(ui, t, 0.0..=1.0, 0.005, "");
            widgets::color_rgba(ui, c);
            if ui.small_button("✖").on_hover_text("Remove stop").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        stops.remove(i);
    }
    if ui.small_button("+ Add stop").clicked() {
        let t = stops.last().map_or(0.0, |(t, _)| (*t + 1.0) * 0.5).min(1.0);
        stops.push((t, stops.last().map_or([1.0; 4], |s| s.1)));
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
}

/// Size multiplier keys: a plot and one row per key.
fn curve_ui(ui: &mut Ui, keys: &mut Vec<(f32, f32)>) {
    let pal = theme::palette(ui.ctx());
    let width = super::inspector::cell_width(ui).clamp(80.0, 400.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 56.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, pal.surface);
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, pal.line),
        egui::StrokeKind::Inside,
    );
    let top = keys.iter().map(|k| k.1).fold(1.0f32, f32::max);
    let map = |t: f32, v: f32| {
        Pos2::new(
            rect.left() + t.clamp(0.0, 1.0) * rect.width(),
            rect.bottom() - 4.0 - (v / top).clamp(0.0, 1.0) * (rect.height() - 8.0),
        )
    };
    if keys.is_empty() {
        painter.line_segment([map(0.0, 1.0), map(1.0, 1.0)], Stroke::new(1.5, pal.accent));
    } else {
        let mut pts = vec![map(0.0, keys[0].1)];
        pts.extend(keys.iter().map(|k| map(k.0, k.1)));
        pts.push(map(1.0, keys[keys.len() - 1].1));
        painter.add(egui::Shape::line(pts, Stroke::new(1.5, pal.accent)));
        for k in keys.iter() {
            painter.circle_filled(map(k.0, k.1), 3.0, pal.text);
        }
    }
    ui.weak(if keys.is_empty() {
        "constant"
    } else {
        "multiplier of the start size"
    });
    let mut remove = None;
    for (i, (t, v)) in keys.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            drag(ui, t, 0.0..=1.0, 0.005, "");
            drag(ui, v, 0.0..=100.0, 0.01, "×");
            if ui.small_button("✖").on_hover_text("Remove key").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        keys.remove(i);
    }
    if ui.small_button("+ Add key").clicked() {
        let t = keys.last().map_or(0.0, |k| (k.0 + 1.0) * 0.5).min(1.0);
        keys.push((t, keys.last().map_or(1.0, |k| k.1)));
    }
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
}
