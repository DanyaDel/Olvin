use egui::{DragValue, RichText, TextEdit};
use glam::Vec3;
use olvin_scene::file::{CameraDesc, MeshDesc, Shape};

use crate::{EditorState, Panel, theme};

#[derive(Default)]
pub struct Inspector;

impl Panel for Inspector {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let Some(id) = state.selection else {
            ui.centered_and_justified(|ui| ui.weak("Ничего не выбрано"));
            return;
        };
        let Some(e) = state.scene.get_mut(id) else { return };

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(TextEdit::singleline(&mut e.name).desired_width(ui.available_width() - 40.0));
                ui.weak(format!("#{id}"));
            });
            ui.add_space(4.0);

            component(ui, "Transform", None, |ui| {
                egui::Grid::new("transform").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("Позиция");
                    vec3(ui, &mut e.position, 0.05, "");
                    ui.end_row();
                    ui.label("Поворот");
                    vec3(ui, &mut e.rotation, 0.5, "°");
                    ui.end_row();
                    ui.label("Масштаб");
                    vec3(ui, &mut e.scale, 0.02, "");
                    ui.end_row();
                });
            });

            let mut remove_mesh = false;
            if let Some(mesh) = &mut e.mesh {
                component(ui, "Mesh Renderer", Some(&mut remove_mesh), |ui| {
                    egui::Grid::new("mesh").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                        ui.label("Форма");
                        egui::ComboBox::from_id_salt("shape")
                            .selected_text(shape_name(mesh.shape))
                            .show_ui(ui, |ui| {
                                for shape in [Shape::Cube, Shape::Plane] {
                                    ui.selectable_value(&mut mesh.shape, shape, shape_name(shape));
                                }
                            });
                        ui.end_row();
                        ui.label("Цвет");
                        ui.color_edit_button_rgba_unmultiplied(&mut mesh.color);
                        ui.end_row();
                    });
                });
            }
            if remove_mesh {
                e.mesh = None;
            }

            let mut remove_camera = false;
            if let Some(cam) = &mut e.camera {
                component(ui, "Камера", Some(&mut remove_camera), |ui| {
                    egui::Grid::new("camera").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                        ui.label("Угол обзора");
                        ui.add(DragValue::new(&mut cam.fov_deg).range(10.0..=170.0).suffix("°"));
                        ui.end_row();
                        ui.label("Ближняя");
                        ui.add(DragValue::new(&mut cam.near).range(0.001..=100.0).speed(0.01));
                        ui.end_row();
                        ui.label("Дальняя");
                        ui.add(DragValue::new(&mut cam.far).range(1.0..=100_000.0));
                        ui.end_row();
                    });
                    ui.weak("Игра смотрит через первую камеру сцены");
                });
            }
            if remove_camera {
                e.camera = None;
            }

            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.menu_button("Добавить компонент", |ui| {
                    if e.mesh.is_none() && ui.button("Mesh Renderer").clicked() {
                        e.mesh = Some(MeshDesc { shape: Shape::Cube, color: [0.8, 0.8, 0.8, 1.0] });
                    }
                    if e.camera.is_none() && ui.button("Камера").clicked() {
                        e.camera = Some(CameraDesc::default());
                    }
                    if e.mesh.is_some() && e.camera.is_some() {
                        ui.weak("Все компоненты уже добавлены");
                    }
                });
            });
        });
    }
}

fn component(ui: &mut egui::Ui, title: &str, remove: Option<&mut bool>, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::group(ui.style()).fill(ui.visuals().faint_bg_color).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong());
            if let Some(remove) = remove {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✖").on_hover_text("Убрать компонент").clicked() {
                        *remove = true;
                    }
                });
            }
        });
        ui.add_space(2.0);
        body(ui);
    });
    ui.add_space(4.0);
}

fn vec3(ui: &mut egui::Ui, v: &mut Vec3, speed: f64, suffix: &str) {
    ui.horizontal(|ui| {
        let width = ((ui.available_width() - 3.0 * 14.0) / 3.0).max(40.0);
        for (i, value) in [&mut v.x, &mut v.y, &mut v.z].into_iter().enumerate() {
            ui.label(RichText::new(["X", "Y", "Z"][i]).color(theme::AXES[i]).strong());
            ui.add_sized(
                [width, 18.0],
                DragValue::new(value).speed(speed).max_decimals(3).suffix(suffix),
            );
        }
    });
}

fn shape_name(shape: Shape) -> &'static str {
    match shape {
        Shape::Cube => "Куб",
        Shape::Plane => "Плоскость",
    }
}
