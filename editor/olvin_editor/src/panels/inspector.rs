use egui::{DragValue, RichText, TextEdit};
use glam::Vec3;
use olvin_scene::file::{
    AudioSourceDesc, BodyKind, CameraDesc, CharacterControllerDesc, ColliderDesc, ColliderShape,
    EntityDesc, LightDesc, LightKind, MeshDesc, RigidbodyDesc, ScriptDesc, Shape,
};

use crate::ext::ComponentEditor;
use crate::{EditorState, Panel, theme};

const TAGS: [&str; 7] = [
    "Untagged",
    "Respawn",
    "Finish",
    "EditorOnly",
    "MainCamera",
    "Player",
    "GameController",
];
const LAYERS: [&str; 6] = [
    "Default",
    "TransparentFX",
    "Ignore Raycast",
    "Ground",
    "Water",
    "UI",
];

#[derive(Default)]
pub struct Inspector {
    search: String,
}

/// A built-in component in the Add Component menu.
struct Builtin {
    category: &'static str,
    name: &'static str,
    has: fn(&EntityDesc) -> bool,
    add: fn(&mut EntityDesc),
}

fn collider_is(e: &EntityDesc, f: fn(&ColliderShape) -> bool) -> bool {
    e.collider.as_ref().is_some_and(|c| f(&c.shape))
}

const BUILTINS: [Builtin; 12] = [
    Builtin {
        category: "Rendering",
        name: "Mesh Renderer",
        has: |e| e.mesh.is_some(),
        add: |e| {
            e.mesh = Some(MeshDesc {
                shape: Shape::Cube,
                color: [0.8; 4],
            })
        },
    },
    Builtin {
        category: "Rendering",
        name: "Camera",
        has: |e| e.camera.is_some(),
        add: |e| e.camera = Some(CameraDesc::default()),
    },
    Builtin {
        category: "Rendering",
        name: "Light",
        has: |e| e.light.is_some(),
        add: |e| e.light = Some(LightDesc::new(LightKind::Point)),
    },
    Builtin {
        category: "Physics",
        name: "Box Collider",
        has: |e| collider_is(e, |s| matches!(s, ColliderShape::Box { .. })),
        add: |e| e.collider = Some(ColliderDesc::new(ColliderShape::Box { size: Vec3::ONE })),
    },
    Builtin {
        category: "Physics",
        name: "Sphere Collider",
        has: |e| collider_is(e, |s| matches!(s, ColliderShape::Sphere { .. })),
        add: |e| e.collider = Some(ColliderDesc::new(ColliderShape::Sphere { radius: 0.5 })),
    },
    Builtin {
        category: "Physics",
        name: "Capsule Collider",
        has: |e| collider_is(e, |s| matches!(s, ColliderShape::Capsule { .. })),
        add: |e| {
            e.collider = Some(ColliderDesc::new(ColliderShape::Capsule {
                radius: 0.5,
                height: 2.0,
            }))
        },
    },
    Builtin {
        category: "Physics",
        name: "Mesh Collider",
        has: |e| collider_is(e, |s| matches!(s, ColliderShape::Mesh)),
        add: |e| e.collider = Some(ColliderDesc::new(ColliderShape::Mesh)),
    },
    Builtin {
        category: "Physics",
        name: "Rigidbody",
        has: |e| e.rigidbody.is_some(),
        add: |e| e.rigidbody = Some(RigidbodyDesc::default()),
    },
    Builtin {
        category: "Physics",
        name: "Character Controller",
        has: |e| e.character_controller.is_some(),
        add: |e| e.character_controller = Some(CharacterControllerDesc::default()),
    },
    Builtin {
        category: "Audio",
        name: "Audio Source",
        has: |e| e.audio_source.is_some(),
        add: |e| e.audio_source = Some(AudioSourceDesc::default()),
    },
    Builtin {
        category: "Audio",
        name: "Audio Listener",
        has: |e| e.audio_listener,
        add: |e| e.audio_listener = true,
    },
    Builtin {
        category: "Scripts",
        name: "New Script",
        has: |_| false,
        add: |e| {
            e.scripts.push(ScriptDesc {
                name: "NewBehaviour".into(),
                ..Default::default()
            })
        },
    },
];

impl Panel for Inspector {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let Some(id) = state.selection else {
            ui.centered_and_justified(|ui| ui.weak("Nothing selected"));
            return;
        };
        let components = state.ext.components.clone();
        let Some(e) = state.scene.get_mut(id) else {
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                header(ui, e, id);
                ui.add_space(4.0);

                section(ui, "⊹ Transform", None, |ui| {
                    grid(ui, "transform", |ui| {
                        row(ui, "Position", |ui| vec3(ui, &mut e.position, 0.05, ""));
                        row(ui, "Rotation", |ui| vec3(ui, &mut e.rotation, 0.5, "°"));
                        row(ui, "Scale", |ui| vec3(ui, &mut e.scale, 0.02, ""));
                    });
                });

                let mut remove = None::<&'static str>;
                if let Some(mesh) = &mut e.mesh {
                    section(ui, "◼ Mesh Renderer", Some((&mut remove, "mesh")), |ui| {
                        grid(ui, "mesh", |ui| {
                            row(ui, "Mesh", |ui| {
                                egui::ComboBox::from_id_salt("shape")
                                    .selected_text(mesh.shape.label())
                                    .show_ui(ui, |ui| {
                                        for shape in Shape::ALL {
                                            ui.selectable_value(
                                                &mut mesh.shape,
                                                shape,
                                                shape.label(),
                                            );
                                        }
                                    });
                            });
                            row(ui, "Color", |ui| {
                                ui.color_edit_button_rgba_unmultiplied(&mut mesh.color);
                            });
                        });
                    });
                }
                if let Some(cam) = &mut e.camera {
                    section(ui, "🎥 Camera", Some((&mut remove, "camera")), |ui| {
                        grid(ui, "camera", |ui| {
                            row(ui, "Field of View", |ui| {
                                drag(ui, &mut cam.fov_deg, 10.0..=170.0, 0.5, "°")
                            });
                            row(ui, "Near", |ui| {
                                drag(ui, &mut cam.near, 0.001..=100.0, 0.01, "")
                            });
                            row(ui, "Far", |ui| {
                                drag(ui, &mut cam.far, 1.0..=100_000.0, 1.0, "")
                            });
                        });
                        ui.weak("The game renders through the first camera");
                    });
                }
                if let Some(light) = &mut e.light {
                    section(ui, "☀ Light", Some((&mut remove, "light")), |ui| {
                        light_ui(ui, light)
                    });
                }
                if let Some(c) = &mut e.collider {
                    let title = match c.shape {
                        ColliderShape::Box { .. } => "▣ Box Collider",
                        ColliderShape::Sphere { .. } => "● Sphere Collider",
                        ColliderShape::Capsule { .. } => "⬭ Capsule Collider",
                        ColliderShape::Mesh => "◬ Mesh Collider",
                    };
                    section(ui, title, Some((&mut remove, "collider")), |ui| {
                        collider_ui(ui, c)
                    });
                }
                if let Some(rb) = &mut e.rigidbody {
                    section(ui, "⚛ Rigidbody", Some((&mut remove, "rigidbody")), |ui| {
                        rigidbody_ui(ui, rb)
                    });
                }
                if let Some(cc) = &mut e.character_controller {
                    section(
                        ui,
                        "🚶 Character Controller",
                        Some((&mut remove, "cc")),
                        |ui| {
                            grid(ui, "cc", |ui| {
                                row(ui, "Height", |ui| {
                                    drag(ui, &mut cc.height, 0.01..=100.0, 0.01, "")
                                });
                                row(ui, "Radius", |ui| {
                                    drag(ui, &mut cc.radius, 0.01..=50.0, 0.01, "")
                                });
                                row(ui, "Step Offset", |ui| {
                                    drag(ui, &mut cc.step_offset, 0.0..=10.0, 0.01, "")
                                });
                                row(ui, "Slope Limit", |ui| {
                                    drag(ui, &mut cc.slope_limit_deg, 0.0..=90.0, 0.5, "°")
                                });
                            });
                        },
                    );
                }
                if let Some(a) = &mut e.audio_source {
                    section(ui, "🔊 Audio Source", Some((&mut remove, "audio")), |ui| {
                        audio_ui(ui, a)
                    });
                }
                if e.audio_listener {
                    section(
                        ui,
                        "🎧 Audio Listener",
                        Some((&mut remove, "listener")),
                        |ui| {
                            ui.weak(
                                "Hears audio sources; keep one per scene (usually on the camera).",
                            );
                        },
                    );
                }
                let mut drop_script = None;
                for (i, script) in e.scripts.iter_mut().enumerate() {
                    let mut gone = None::<&'static str>;
                    section(ui, "📜 Script", Some((&mut gone, "script")), |ui| {
                        script_ui(ui, script, i)
                    });
                    if gone.is_some() {
                        drop_script = Some(i);
                    }
                }
                if let Some(i) = drop_script {
                    e.scripts.remove(i);
                }
                match remove {
                    Some("mesh") => e.mesh = None,
                    Some("camera") => e.camera = None,
                    Some("light") => e.light = None,
                    Some("collider") => e.collider = None,
                    Some("rigidbody") => e.rigidbody = None,
                    Some("cc") => e.character_controller = None,
                    Some("audio") => e.audio_source = None,
                    Some("listener") => e.audio_listener = false,
                    _ => {}
                }

                for c in &components {
                    if (c.has)(e) {
                        let mut gone = None::<&'static str>;
                        section(ui, c.title, Some((&mut gone, "ext")), |ui| (c.ui)(ui, e));
                        if gone.is_some() {
                            (c.remove)(e);
                        }
                    }
                }

                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    let button = egui::Button::new(RichText::new("Add Component").strong())
                        .min_size(egui::vec2(220.0, 26.0));
                    let response = ui.add(button);
                    egui::Popup::menu(&response).show(|ui| self.add_menu(ui, e, &components));
                });
            });
    }
}

impl Inspector {
    fn add_menu(&mut self, ui: &mut egui::Ui, e: &mut EntityDesc, elements: &[ComponentEditor]) {
        ui.set_min_width(230.0);
        ui.add(
            TextEdit::singleline(&mut self.search)
                .hint_text("🔍 Search")
                .desired_width(220.0),
        )
        .request_focus();
        let filter = self.search.to_lowercase();
        let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
        let mut added = false;
        for category in ["Rendering", "Physics", "Audio", "Scripts"] {
            let items: Vec<&Builtin> = BUILTINS
                .iter()
                .filter(|b| b.category == category && matches(b.name))
                .collect();
            if items.is_empty() {
                continue;
            }
            ui.label(RichText::new(category).small().weak());
            for b in items {
                let has = (b.has)(e);
                let replaces = b.name.ends_with("Collider") && e.collider.is_some() && !has;
                let label = if replaces {
                    format!("{}  (replaces collider)", b.name)
                } else {
                    b.name.to_owned()
                };
                if ui.add_enabled(!has, egui::Button::new(label)).clicked() {
                    (b.add)(e);
                    added = true;
                }
            }
        }
        let ext: Vec<&ComponentEditor> = elements.iter().filter(|c| matches(c.title)).collect();
        if !ext.is_empty() {
            ui.label(RichText::new("Elements").small().weak());
            for c in ext {
                if ui
                    .add_enabled(!(c.has)(e), egui::Button::new(c.title))
                    .clicked()
                {
                    (c.add)(e);
                    added = true;
                }
            }
        }
        if added {
            self.search.clear();
            ui.close();
        }
    }
}

fn header(ui: &mut egui::Ui, e: &mut EntityDesc, id: u64) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut e.active, "")
            .on_hover_text("Active: inactive objects and their children are not spawned");
        ui.add(TextEdit::singleline(&mut e.name).desired_width(ui.available_width() - 40.0));
        ui.weak(format!("#{id}"));
    });
    ui.horizontal(|ui| {
        ui.label("Tag");
        let tag = if e.tag.is_empty() {
            "Untagged"
        } else {
            e.tag.as_str()
        };
        egui::ComboBox::from_id_salt("tag")
            .width(110.0)
            .selected_text(tag)
            .show_ui(ui, |ui| {
                for t in TAGS {
                    let value = if t == "Untagged" {
                        String::new()
                    } else {
                        t.to_owned()
                    };
                    ui.selectable_value(&mut e.tag, value, t);
                }
            });
        ui.label("Layer");
        let layer = LAYERS
            .get(e.layer as usize)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Layer {}", e.layer));
        egui::ComboBox::from_id_salt("layer")
            .width(110.0)
            .selected_text(layer)
            .show_ui(ui, |ui| {
                for (i, l) in LAYERS.iter().enumerate() {
                    ui.selectable_value(&mut e.layer, i as u8, *l);
                }
                for i in LAYERS.len() as u8..32 {
                    ui.selectable_value(&mut e.layer, i, format!("Layer {i}"));
                }
            });
    });
}

fn light_ui(ui: &mut egui::Ui, l: &mut LightDesc) {
    grid(ui, "light", |ui| {
        row(ui, "Type", |ui| {
            egui::ComboBox::from_id_salt("light-kind")
                .selected_text(format!("{:?}", l.kind))
                .show_ui(ui, |ui| {
                    for k in [LightKind::Directional, LightKind::Point, LightKind::Spot] {
                        ui.selectable_value(&mut l.kind, k, format!("{k:?}"));
                    }
                });
        });
        row(ui, "Color", |ui| {
            ui.color_edit_button_rgb(&mut l.color);
        });
        row(ui, "Intensity", |ui| {
            drag(ui, &mut l.intensity, 0.0..=100.0, 0.01, "")
        });
        if l.kind != LightKind::Directional {
            row(ui, "Range", |ui| {
                drag(ui, &mut l.range, 0.0..=1000.0, 0.1, "")
            });
        }
        if l.kind == LightKind::Spot {
            row(ui, "Spot Angle", |ui| {
                drag(ui, &mut l.spot_angle, 1.0..=179.0, 0.5, "°")
            });
        }
        row(ui, "Shadows", |ui| {
            ui.checkbox(&mut l.shadows, "");
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.weak("Temperature");
        for (k, label) in [
            (1900.0, "Candle"),
            (2700.0, "Bulb"),
            (4000.0, "Neutral"),
            (5500.0, "Sun"),
            (7500.0, "Sky"),
        ] {
            let c = kelvin_to_rgb(k);
            let fill = egui::Color32::from_rgb(
                (c[0].powf(1.0 / 2.2) * 255.0) as u8,
                (c[1].powf(1.0 / 2.2) * 255.0) as u8,
                (c[2].powf(1.0 / 2.2) * 255.0) as u8,
            );
            let b = egui::Button::new(
                egui::RichText::new(label)
                    .color(egui::Color32::from_gray(20))
                    .small(),
            )
            .fill(fill);
            if ui.add(b).on_hover_text(format!("{k} K")).clicked() {
                l.color = c;
            }
        }
    });
    if l.kind != LightKind::Directional && l.shadows {
        ui.weak("Only the sun (first Directional light) casts shadows for now.");
    }
}

/// Linear RGB of a black body at `k` kelvin (Tanner Helland's fit), max channel 1.
fn kelvin_to_rgb(k: f32) -> [f32; 3] {
    let t = k / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.7 * (t - 60.0).powf(-0.1332)
    };
    let g = if t <= 66.0 {
        99.47 * t.ln() - 161.12
    } else {
        288.12 * (t - 60.0).powf(-0.0755)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.52 * (t - 10.0).ln() - 305.04
    };
    // The fit is in sRGB; the scene stores linear colour.
    [r, g, b].map(|c| (c.clamp(0.0, 255.0) / 255.0).powf(2.2))
}

fn collider_ui(ui: &mut egui::Ui, c: &mut ColliderDesc) {
    grid(ui, "collider", |ui| {
        row(ui, "Is Trigger", |ui| {
            ui.checkbox(&mut c.is_trigger, "");
        });
        if c.shape != ColliderShape::Mesh {
            row(ui, "Center", |ui| vec3(ui, &mut c.center, 0.02, ""));
        }
        match &mut c.shape {
            ColliderShape::Box { size } => row(ui, "Size", |ui| vec3(ui, size, 0.02, "")),
            ColliderShape::Sphere { radius } => {
                row(ui, "Radius", |ui| drag(ui, radius, 0.0..=1000.0, 0.01, ""))
            }
            ColliderShape::Capsule { radius, height } => {
                row(ui, "Radius", |ui| drag(ui, radius, 0.0..=1000.0, 0.01, ""));
                row(ui, "Height", |ui| drag(ui, height, 0.0..=1000.0, 0.01, ""));
            }
            ColliderShape::Mesh => row(ui, "Mesh", |ui| {
                ui.weak("from Mesh Renderer / CSG level");
            }),
        }
        row(ui, "Friction", |ui| {
            drag(ui, &mut c.friction, 0.0..=2.0, 0.01, "")
        });
        row(ui, "Bounciness", |ui| {
            drag(ui, &mut c.restitution, 0.0..=1.0, 0.01, "")
        });
    });
}

fn rigidbody_ui(ui: &mut egui::Ui, rb: &mut RigidbodyDesc) {
    grid(ui, "rigidbody", |ui| {
        row(ui, "Body Type", |ui| {
            ui.selectable_value(&mut rb.kind, BodyKind::Dynamic, "Dynamic");
            ui.selectable_value(&mut rb.kind, BodyKind::Kinematic, "Kinematic");
        });
        row(ui, "Mass", |ui| {
            drag(ui, &mut rb.mass, 0.0001..=1e6, 0.05, "")
        });
        row(ui, "Drag", |ui| {
            drag(ui, &mut rb.drag, 0.0..=100.0, 0.01, "")
        });
        row(ui, "Angular Drag", |ui| {
            drag(ui, &mut rb.angular_drag, 0.0..=100.0, 0.01, "")
        });
        row(ui, "Gravity Scale", |ui| {
            drag(ui, &mut rb.gravity_scale, -10.0..=10.0, 0.01, "")
        });
        row(ui, "Freeze Rotation", |ui| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                ui.checkbox(&mut rb.freeze_rotation[i], *axis);
            }
        });
        row(ui, "Continuous", |ui| {
            ui.checkbox(&mut rb.ccd, "")
                .on_hover_text("Continuous collision detection for fast objects");
        });
    });
}

fn audio_ui(ui: &mut egui::Ui, a: &mut AudioSourceDesc) {
    grid(ui, "audio", |ui| {
        row(ui, "Clip", |ui| {
            ui.add(
                TextEdit::singleline(&mut a.clip)
                    .hint_text("sounds/step.ogg")
                    .desired_width(160.0),
            );
        });
        row(ui, "Volume", |ui| {
            ui.add(egui::Slider::new(&mut a.volume, 0.0..=1.0));
        });
        row(ui, "Pitch", |ui| {
            ui.add(egui::Slider::new(&mut a.pitch, -3.0..=3.0));
        });
        row(ui, "Loop", |ui| {
            ui.checkbox(&mut a.looping, "");
        });
        row(ui, "Play On Awake", |ui| {
            ui.checkbox(&mut a.play_on_awake, "");
        });
        row(ui, "3D Sound", |ui| {
            ui.checkbox(&mut a.spatial, "");
        });
        if a.spatial {
            row(ui, "Min Distance", |ui| {
                drag(ui, &mut a.min_distance, 0.0..=1e4, 0.05, "")
            });
            row(ui, "Max Distance", |ui| {
                drag(ui, &mut a.max_distance, 0.0..=1e5, 0.5, "")
            });
        }
    });
}

fn script_ui(ui: &mut egui::Ui, script: &mut ScriptDesc, index: usize) {
    ui.horizontal(|ui| {
        ui.label("Behaviour");
        ui.add(TextEdit::singleline(&mut script.name).desired_width(f32::INFINITY));
    });
    let mut remove = None;
    let mut rename = None;
    egui::Grid::new(("script", index))
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for (key, value) in script.params.iter_mut() {
                let mut k = key.clone();
                if ui
                    .add(TextEdit::singleline(&mut k).desired_width(90.0))
                    .lost_focus()
                    && k != *key
                {
                    rename = Some((key.clone(), k));
                }
                ui.add(TextEdit::singleline(value).desired_width(120.0));
                if ui.small_button("✖").clicked() {
                    remove = Some(key.clone());
                }
                ui.end_row();
            }
        });
    if let Some(k) = remove {
        script.params.remove(&k);
    }
    if let Some((old, new)) = rename
        && !new.is_empty()
        && let Some(v) = script.params.remove(&old)
    {
        script.params.insert(new, v);
    }
    if ui.small_button("+ Parameter").clicked() {
        let mut n = script.params.len();
        while script.params.contains_key(&format!("param{n}")) {
            n += 1;
        }
        script.params.insert(format!("param{n}"), String::new());
    }
}

/// A component block; `remove` gets `Some(key)` when its ✖ is clicked.
pub(crate) fn section(
    ui: &mut egui::Ui,
    title: &str,
    remove: Option<(&mut Option<&'static str>, &'static str)>,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let id = ui.make_persistent_id(title);
            let mut open = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                true,
            );
            ui.horizontal(|ui| {
                open.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
                ui.label(RichText::new(title).strong());
                if let Some((slot, key)) = remove {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✖")
                            .on_hover_text("Remove component")
                            .clicked()
                        {
                            *slot = Some(key);
                        }
                    });
                }
            });
            open.show_body_unindented(ui, |ui| {
                ui.add_space(2.0);
                body(ui);
            });
            open.store(ui.ctx());
        });
    ui.add_space(4.0);
}

pub(crate) fn grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([10.0, 4.0])
        .min_col_width(96.0)
        .show(ui, body);
}

pub(crate) fn row(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(label);
    ui.horizontal(body);
    ui.end_row();
}

pub(crate) fn drag(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    suffix: &str,
) {
    ui.add(
        DragValue::new(value)
            .range(range)
            .speed(speed)
            .max_decimals(3)
            .suffix(suffix),
    );
}

fn vec3(ui: &mut egui::Ui, v: &mut Vec3, speed: f64, suffix: &str) {
    let width = ((ui.available_width() - 3.0 * 14.0) / 3.0).clamp(40.0, 90.0);
    for (i, value) in [&mut v.x, &mut v.y, &mut v.z].into_iter().enumerate() {
        ui.label(
            RichText::new(["X", "Y", "Z"][i])
                .color(theme::AXES[i])
                .strong(),
        );
        ui.add_sized(
            [width, 18.0],
            DragValue::new(value)
                .speed(speed)
                .max_decimals(3)
                .suffix(suffix),
        );
    }
}
