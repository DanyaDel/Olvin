use egui::{Color32, DragValue, RichText, TextEdit};
use glam::Vec3;
use olvin_scene::action::{Action, KeyState, Prop};
use olvin_scene::file::{
    AudioSourceDesc, BodyKind, CameraDesc, CharacterControllerDesc, ColliderDesc, ColliderShape,
    EntityDesc, JointDesc, JointKind, LightDesc, LightKind, MeshDesc, RigidbodyDesc, ScriptDesc,
    Shape,
};

use crate::ext::ComponentEditor;
use crate::{EditorState, Panel, PrefabReport, theme};

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

/// The fields of the selected prefab node that differ from the asset, and the
/// reverts asked for. Shared with `section`/`row_field` through the egui
/// context while the Inspector draws, so every component block can mark itself
/// (blue title, ↺) without its own parameters.
#[derive(Clone, Default)]
struct PrefabMarks {
    overridden: std::sync::Arc<Vec<String>>,
    reverts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

fn marks_id() -> egui::Id {
    egui::Id::new("inspector-prefab-marks")
}

fn marks(ui: &egui::Ui) -> Option<PrefabMarks> {
    ui.ctx().data(|d| d.get_temp::<PrefabMarks>(marks_id()))
}

impl PrefabMarks {
    fn is_overridden(&self, field: &str) -> bool {
        self.overridden.iter().any(|k| k == field)
    }

    fn revert(&self, field: &str) {
        if let Ok(mut list) = self.reverts.lock() {
            list.push(field.to_owned());
        }
    }
}

/// The override key of the field a component block shows (its remove key).
fn field_of(key: &str) -> Option<&'static str> {
    Some(match key {
        "mesh" => "mesh",
        "model" => "model",
        "camera" => "camera",
        "light" => "light",
        "collider" => "collider",
        "rigidbody" => "rigidbody",
        "joint" => "joint",
        "cc" => "character_controller",
        "audio" => "audio_source",
        "particles" => "particles",
        "listener" => "audio_listener",
        "script" => "scripts",
        "material" => "material",
        _ => return None,
    })
}

/// What the prefab header asks the state to do.
enum PrefabAction {
    OpenAsset(String),
    SelectAsset(String),
    SelectRoot(u64),
    RevertAll(u64),
    ApplyAll(u64),
    Unpack(u64),
    RevertField(String),
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

const BUILTINS: [Builtin; 19] = [
    Builtin {
        category: "Rendering",
        name: "Model",
        has: |e| e.model.is_some(),
        add: |e| e.model = Some(olvin_scene::file::ModelDesc::new("models/model.gltf")),
    },
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
        name: "Particle System",
        has: |e| e.particles.is_some(),
        add: |e| e.particles = Some(olvin_scene::file::ParticleDesc::default()),
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
        name: "Fixed Joint",
        has: |e| matches!(&e.joint, Some(j) if j.kind == JointKind::Fixed),
        add: |e| e.joint = Some(JointDesc::new(JointKind::Fixed)),
    },
    Builtin {
        category: "Physics",
        name: "Hinge Joint",
        has: |e| matches!(&e.joint, Some(j) if matches!(j.kind, JointKind::Hinge { .. })),
        add: |e| {
            e.joint = Some(JointDesc::new(JointKind::Hinge {
                axis: Vec3::Y,
                limits: None,
            }))
        },
    },
    Builtin {
        category: "Physics",
        name: "Ball Joint",
        has: |e| matches!(&e.joint, Some(j) if j.kind == JointKind::Ball),
        add: |e| e.joint = Some(JointDesc::new(JointKind::Ball)),
    },
    Builtin {
        category: "Physics",
        name: "Slider Joint",
        has: |e| matches!(&e.joint, Some(j) if matches!(j.kind, JointKind::Slider { .. })),
        add: |e| {
            e.joint = Some(JointDesc::new(JointKind::Slider {
                axis: Vec3::X,
                limits: None,
            }))
        },
    },
    Builtin {
        category: "Physics",
        name: "Spring Joint",
        has: |e| matches!(&e.joint, Some(j) if matches!(j.kind, JointKind::Spring { .. })),
        add: |e| {
            e.joint = Some(JointDesc::new(JointKind::Spring {
                rest_length: 1.0,
                stiffness: 50.0,
                damping: 5.0,
            }))
        },
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
        let assets = state.assets_dir();
        state.materials.refresh(assets.as_deref());
        let report = state.prefab_report(id);
        let prefab_marks = PrefabMarks {
            overridden: std::sync::Arc::new(
                report
                    .as_ref()
                    .map(|r| r.overridden.clone())
                    .unwrap_or_default(),
            ),
            ..Default::default()
        };
        ui.ctx()
            .data_mut(|d| d.insert_temp(marks_id(), prefab_marks.clone()));
        let mut prefab_actions = Vec::new();
        let mut log = None;
        let frame = state.anim_frame as f32;
        let clips = state
            .scene
            .get(id)
            .and_then(|e| e.model.as_ref())
            .map(|m| m.path.clone())
            .map(|path| state.engine_scene.model_clips(assets.clone(), &path))
            .unwrap_or_default();
        // The other objects a joint may connect to (named in the Connected list).
        let joint_targets: Vec<(u64, String)> = state
            .scene
            .get(id)
            .filter(|e| e.joint.is_some())
            .map(|_| {
                state
                    .scene
                    .entities
                    .iter()
                    .filter(|o| o.id != id)
                    .map(|o| (o.id, o.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        // Drawn before the scene is borrowed apart; a change made this frame
        // shows on the next one (the section asks for it).
        let material_preview = state
            .scene
            .get(id)
            .filter(|e| e.mesh.is_some() || e.poly.is_some() || e.brush.is_some())
            .and_then(|e| e.material.clone())
            .and_then(|rel| {
                let desc = state.materials.get(&rel)?.clone();
                crate::material_preview::render(state, &rel, &desc)
            });
        let fx = &mut state.engine_scene.fx;
        let olvin_scene::file::SceneFile {
            entities,
            animation,
            ..
        } = &mut state.scene;
        let Some(e) = entities.iter_mut().find(|e| e.id == id) else {
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                header(ui, e, id);
                ui.add_space(4.0);
                if let Some(report) = &report {
                    prefab_header(ui, report, &mut prefab_actions);
                }

                section(ui, "⊹ Transform", None, |ui| {
                    grid(ui, "transform", |ui| {
                        for (label, prop, value, speed, suffix) in [
                            ("Position", Prop::Location, &mut e.position, 0.05, ""),
                            ("Rotation", Prop::Rotation, &mut e.rotation, 0.5, "°"),
                            ("Scale", Prop::Scale, &mut e.scale, 0.02, ""),
                        ] {
                            let field = prop_field(prop);
                            row_field(ui, label, field, |ui| {
                                let keying = Keying {
                                    action: &mut *animation,
                                    object: id,
                                    prop,
                                    frame,
                                };
                                vec3_keyed(ui, value, speed, suffix, keying)
                            });
                        }
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
                                crate::widgets::color_rgba(ui, &mut mesh.color);
                            });
                        });
                    });
                }
                if e.mesh.is_some() || e.poly.is_some() || e.brush.is_some() {
                    log = material_section(ui, e, &mut state.materials, material_preview);
                }
                if let Some(model) = &mut e.model {
                    let preview = state.model_previews.entry(id).or_default();
                    section(ui, "🧍 Model", Some((&mut remove, "model")), |ui| {
                        grid(ui, "model", |ui| {
                            row(ui, "Path", |ui| {
                                ui.text_edit_singleline(&mut model.path);
                            });
                            row(ui, "Animation", |ui| {
                                animation_combo(ui, &mut model.animation, &clips)
                            });
                            if !clips.is_empty() || model.animation.is_some() {
                                row(ui, "Preview", |ui| preview_clip_combo(ui, preview, &clips));
                                row(ui, "Speed", |ui| {
                                    drag(ui, &mut preview.speed, 0.0..=8.0, 0.02, "×")
                                });
                                row(ui, "Loop", |ui| {
                                    ui.checkbox(&mut preview.looping, "");
                                });
                                row(ui, "", |ui| {
                                    let label = if preview.playing {
                                        "■ Stop preview"
                                    } else {
                                        "▶ Play preview"
                                    };
                                    if ui.button(label).clicked() {
                                        preview.playing = !preview.playing;
                                    }
                                });
                            }
                        });
                        ui.weak(
                            "The scene view shows the bind pose until a preview plays \
                             (preview settings are not saved); Play runs the scene's animation",
                        );
                    });
                }
                if let Some(t) = &mut e.terrain {
                    let textures = state.materials.textures.clone();
                    section(ui, "▲ Terrain", Some((&mut remove, "terrain")), |ui| {
                        terrain_ui(ui, t, &textures)
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
                if let Some(p) = &mut e.particles {
                    section(
                        ui,
                        "✨ Particle System",
                        Some((&mut remove, "particles")),
                        |ui| {
                            let preview = super::particles::Preview { fx, id };
                            super::particles::particle_ui(ui, p, Some(preview));
                        },
                    );
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
                if let Some(j) = &mut e.joint {
                    let title = match j.kind {
                        JointKind::Fixed => "🔗 Fixed Joint",
                        JointKind::Hinge { .. } => "🔗 Hinge Joint",
                        JointKind::Ball => "🔗 Ball Joint",
                        JointKind::Slider { .. } => "🔗 Slider Joint",
                        JointKind::Spring { .. } => "🔗 Spring Joint",
                    };
                    section(ui, title, Some((&mut remove, "joint")), |ui| {
                        joint_ui(ui, j, &joint_targets)
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
                    // Each script keeps its own collapsed state.
                    let title = if i == 0 {
                        "📜 Script".to_owned()
                    } else {
                        format!("📜 Script {}", i + 1)
                    };
                    section(ui, &title, Some((&mut gone, "script")), |ui| {
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
                    Some("model") => e.model = None,
                    Some("camera") => e.camera = None,
                    Some("light") => e.light = None,
                    Some("particles") => e.particles = None,
                    Some("collider") => e.collider = None,
                    Some("rigidbody") => e.rigidbody = None,
                    Some("joint") => e.joint = None,
                    Some("terrain") => e.terrain = None,
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
        ui.ctx()
            .data_mut(|d| d.remove_temp::<PrefabMarks>(marks_id()));
        match log {
            Some(Ok(msg)) => state.info(msg),
            Some(Err(msg)) => state.warn(msg),
            None => {}
        }
        let reverts = prefab_marks
            .reverts
            .lock()
            .map(|l| l.clone())
            .unwrap_or_default();
        prefab_actions.extend(reverts.into_iter().map(PrefabAction::RevertField));
        for action in prefab_actions {
            match action {
                PrefabAction::OpenAsset(asset) => state.open_prefab_asset(&asset),
                PrefabAction::SelectAsset(asset) => state.select_prefab_asset(&asset),
                PrefabAction::SelectRoot(root) => state.select(Some(root)),
                PrefabAction::RevertAll(root) => state.revert_prefab(root),
                PrefabAction::ApplyAll(root) => {
                    if let Err(err) = state.apply_prefab(root) {
                        state.warn(format!("apply prefab: {err}"));
                    }
                }
                PrefabAction::Unpack(root) => state.unpack_prefab(root),
                PrefabAction::RevertField(key) => state.revert_prefab_field(id, &key),
            }
        }
    }
}

/// The override key of a transform property.
fn prop_field(prop: Prop) -> &'static str {
    match prop {
        Prop::Location => "position",
        Prop::Rotation => "rotation",
        Prop::Scale => "scale",
    }
}

/// [`row`] for a field that can differ from the prefab: the label turns blue and gets a ↺.
fn row_field(ui: &mut egui::Ui, label: &str, field: &str, body: impl FnOnce(&mut egui::Ui)) {
    let marks = marks(ui).filter(|m| m.is_overridden(field));
    match &marks {
        Some(marks) => {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(label)
                        .strong()
                        .color(theme::palette(ui.ctx()).info),
                );
                if ui
                    .small_button("↺")
                    .on_hover_text("Revert to the prefab's value")
                    .clicked()
                {
                    marks.revert(field);
                }
            });
        }
        None => {
            ui.label(label);
        }
    }
    ui.horizontal(body);
    ui.end_row();
}

/// The Prefab block: where the object comes from and what the instance changes.
fn prefab_header(ui: &mut egui::Ui, r: &PrefabReport, actions: &mut Vec<PrefabAction>) {
    let p = theme::palette(ui.ctx());
    let name = r.asset.rsplit('/').next().unwrap_or(&r.asset);
    let name = name.trim_end_matches(".prefab.ron");
    egui::Frame::group(ui.style())
        .fill(p.info.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, p.info.gamma_multiply(0.5)))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("◆").color(p.info).strong());
                ui.label(theme::caps(ui.ctx(), "Prefab").color(p.info));
                ui.add(egui::Label::new(RichText::new(name).strong()).truncate())
                    .on_hover_text(&r.asset);
            });
            if let Some(why) = &r.missing {
                ui.colored_label(p.warn, format!("The prefab could not be read: {why}"));
            } else if !r.is_root {
                ui.weak("Part of a prefab instance; edits here are overrides.");
                if ui.small_button("Select Root").clicked() {
                    actions.push(PrefabAction::SelectRoot(r.root));
                }
            }
            if r.is_root {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button("Open")
                        .on_hover_text("Edit the prefab as a scene")
                        .clicked()
                    {
                        actions.push(PrefabAction::OpenAsset(r.asset.clone()));
                    }
                    if ui
                        .small_button("Select")
                        .on_hover_text("Show the asset in the Assets panel")
                        .clicked()
                    {
                        actions.push(PrefabAction::SelectAsset(r.asset.clone()));
                    }
                    let changed = r.overrides + r.removed + r.added > 0;
                    if ui
                        .add_enabled(changed, egui::Button::new("Revert All").small())
                        .on_hover_text("Drop every override of this instance")
                        .clicked()
                    {
                        actions.push(PrefabAction::RevertAll(r.root));
                    }
                    if ui
                        .add_enabled(changed, egui::Button::new("Apply All").small())
                        .on_hover_text(
                            "Write this instance's changes into the prefab; all instances follow",
                        )
                        .clicked()
                    {
                        actions.push(PrefabAction::ApplyAll(r.root));
                    }
                    if ui
                        .small_button("Unpack")
                        .on_hover_text("Detach from the prefab; the objects stay")
                        .clicked()
                    {
                        actions.push(PrefabAction::Unpack(r.root));
                    }
                });
                if r.missing.is_none() {
                    let total = [
                        (r.overrides, "override"),
                        (r.removed, "removed"),
                        (r.added, "added"),
                    ]
                    .into_iter()
                    .filter(|(n, _)| *n > 0)
                    .map(|(n, what)| {
                        if what == "override" && n != 1 {
                            format!("{n} overrides")
                        } else {
                            format!("{n} {what}")
                        }
                    })
                    .collect::<Vec<_>>();
                    ui.weak(if total.is_empty() {
                        "No overrides".to_owned()
                    } else {
                        total.join(", ")
                    });
                }
            }
            for key in &r.overridden {
                ui.horizontal(|ui| {
                    if ui
                        .small_button("↺")
                        .on_hover_text("Revert to the prefab's value")
                        .clicked()
                    {
                        actions.push(PrefabAction::RevertField(key.clone()));
                    }
                    ui.label(RichText::new(key).color(p.info));
                });
            }
        });
    ui.add_space(4.0);
}

/// The object's material: pick or create a `*.mat.ron`, and edit it in place.
fn material_section(
    ui: &mut egui::Ui,
    e: &mut EntityDesc,
    lib: &mut crate::materials::MaterialLibrary,
    preview: Option<egui::TextureId>,
) -> Option<Result<String, String>> {
    use crate::materials::{material_ui, presets_ui, short_name};
    let mut log = None;
    let mut clear = None::<&'static str>;
    section(ui, "🎨 Material", Some((&mut clear, "material")), |ui| {
        ui.horizontal(|ui| {
            let shown = e
                .material
                .as_deref()
                .map_or("Default (colour only)".to_owned(), short_name);
            let combo = (ui.available_width() - 56.0 - 2.0 * ui.spacing().item_spacing.x)
                .clamp(60.0, 180.0);
            egui::ComboBox::from_id_salt("material-asset")
                .width(combo)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.material, None, "Default (colour only)");
                    for m in &lib.materials {
                        ui.selectable_value(&mut e.material, Some(m.clone()), short_name(m));
                    }
                });
            if ui
                .button("New")
                .on_hover_text("Create materials/<name>.mat.ron from this object's colour")
                .clicked()
            {
                let color = e
                    .mesh
                    .as_ref()
                    .map(|m| m.color)
                    .or(e.brush.as_ref().map(|b| b.color))
                    .unwrap_or([0.8; 4]);
                let desc = olvin_scene::material::MaterialDesc {
                    base_color: color,
                    ..Default::default()
                };
                match lib.create(&e.name, desc) {
                    Ok(rel) => {
                        log = Some(Ok(format!("created {rel}")));
                        e.material = Some(rel);
                    }
                    Err(err) => log = Some(Err(err)),
                }
            }
        });
        let Some(rel) = e.material.clone() else {
            ui.weak("Textures, PBR, emission and custom shaders live in material assets.");
            return;
        };
        let (textures, shaders) = (lib.textures.clone(), lib.shaders.clone());
        let Some(m) = lib.get(&rel) else {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("{rel}: missing or unreadable"),
            );
            return;
        };
        if let Some(texture) = preview {
            let side = crate::material_preview::PREVIEW_PIXELS as f32 / 2.0;
            ui.vertical_centered(|ui| {
                ui.add(
                    egui::Image::new((texture, egui::vec2(side, side)))
                        .corner_radius(6.0)
                        .sense(egui::Sense::hover()),
                )
                .on_hover_text("Preview: this material on a sphere, lit by a studio light with the scene's sky and exposure");
            });
        }
        let mut changed = presets_ui(ui, m);
        changed |= material_ui(ui, m, &textures, &shaders);
        if changed {
            // The preview is drawn before this section: one more frame shows the edit.
            ui.ctx().request_repaint();
        }
        // Stacked: the path and the button do not both fit a narrow panel.
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{rel} · shared by every object using it")).weak(),
                )
                .truncate(),
            );
            if ui
                .small_button("New shader")
                .on_hover_text("shaders/<material>.wgsl from the surface shader template")
                .clicked()
            {
                match lib.create_shader(&short_name(&rel)) {
                    Ok(path) => {
                        if let Some(m) = lib.get(&rel) {
                            m.shader = Some(path.clone());
                        }
                        changed = true;
                        log = Some(Ok(format!("created {path}")));
                    }
                    Err(err) => log = Some(Err(err)),
                }
            }
        });
        if changed && let Err(err) = lib.save(&rel) {
            log = Some(Err(err));
        }
    });
    if clear.is_some() {
        e.material = None;
    }
    log
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
        let presets: Vec<_> = olvin_scene::file::ParticleDesc::PRESETS
            .iter()
            .filter(|(n, _)| matches(n) || matches("particle"))
            .collect();
        if !presets.is_empty() && e.particles.is_none() {
            ui.label(RichText::new("Particle presets").small().weak());
            for (name, make) in presets {
                if ui.button(format!("✨ {name}")).clicked() {
                    e.particles = Some(make());
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
        // Leave room for the "#id" after the field.
        let marks = marks(ui);
        let name_changed = marks.as_ref().filter(|m| m.is_overridden("name"));
        let reserve = if name_changed.is_some() { 66.0 } else { 42.0 };
        let width = cell_width(ui) - reserve - 2.0 * ui.spacing().item_spacing.x;
        ui.add(TextEdit::singleline(&mut e.name).desired_width(width.max(40.0)));
        if let Some(marks) = name_changed
            && ui
                .small_button("↺")
                .on_hover_text("Revert the name to the prefab's")
                .clicked()
        {
            marks.revert("name");
        }
        // A prefab node's id is a hash: not worth showing.
        if olvin_scene::prefab::is_member_id(id) {
            ui.weak("node");
        } else {
            ui.weak(format!("#{id}"));
        }
    });
    ui.horizontal(|ui| {
        // Two combo boxes plus their labels: shrink with the panel, or the
        // overflow widens everything below.
        let combo =
            ((cell_width(ui) - 76.0 - 4.0 * ui.spacing().item_spacing.x) / 2.0).clamp(56.0, 110.0);
        let marks = marks(ui);
        let marked = |ui: &mut egui::Ui, text: &str, field: &str| {
            if marks.as_ref().is_some_and(|m| m.is_overridden(field)) {
                ui.label(
                    RichText::new(text)
                        .strong()
                        .color(theme::palette(ui.ctx()).info),
                );
            } else {
                ui.label(text);
            }
        };
        marked(ui, "Tag", "tag");
        let tag = if e.tag.is_empty() {
            "Untagged"
        } else {
            e.tag.as_str()
        };
        egui::ComboBox::from_id_salt("tag")
            .width(combo)
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
        marked(ui, "Layer", "layer");
        let layer = LAYERS
            .get(e.layer as usize)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Layer {}", e.layer));
        egui::ComboBox::from_id_salt("layer")
            .width(combo)
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
            crate::widgets::color_rgb(ui, &mut l.color);
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
        row(ui, "Cast Shadows", |ui| {
            ui.checkbox(&mut l.shadows, "");
        });
        if l.shadows {
            row(ui, "Shadow Bias", |ui| {
                drag(ui, &mut l.shadow_bias, 0.0..=16.0, 0.05, " texels")
            });
            if l.kind == LightKind::Directional {
                row(ui, "Size", |ui| drag(ui, &mut l.size, 0.0..=60.0, 0.1, "°"));
            }
        }
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
    if l.shadows {
        ui.weak(
            "Bias: raise against acne, lower if shadows detach from their casters. \
             Size: the sun's softness with the PCSS filter.",
        );
    }
    if l.kind != LightKind::Directional && l.shadows {
        ui.weak("Only the most important spot / point lights get a shadow map (Settings → Rendering → Shadows).");
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
        row(ui, "Layer", |ui| {
            let mut layer = u32::from(c.layer);
            if ui
                .add(DragValue::new(&mut layer).range(0..=31).speed(0.1))
                .changed()
            {
                c.layer = layer as u8;
            }
        });
        row(ui, "Collides With", |ui| {
            let count = c.mask.count_ones();
            let label = match count {
                32 => "All layers".to_string(),
                0 => "Nothing".to_string(),
                n => format!("{n} layers"),
            };
            ui.menu_button(label, |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("All").clicked() {
                        c.mask = u32::MAX;
                    }
                    if ui.small_button("None").clicked() {
                        c.mask = 0;
                    }
                });
                egui::Grid::new("collider mask").show(ui, |ui| {
                    for layer in 0..32u32 {
                        let mut on = c.mask & (1 << layer) != 0;
                        if ui.checkbox(&mut on, layer.to_string()).changed() {
                            c.mask ^= 1 << layer;
                        }
                        if layer % 8 == 7 {
                            ui.end_row();
                        }
                    }
                });
            });
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

fn joint_ui(ui: &mut egui::Ui, j: &mut JointDesc, targets: &[(u64, String)]) {
    grid(ui, "joint", |ui| {
        row(ui, "Connected To", |ui| {
            let current = match j.connected {
                None => "World".to_string(),
                Some(id) => targets
                    .iter()
                    .find(|(t, _)| *t == id)
                    .map_or_else(|| format!("missing #{id}"), |(_, n)| n.clone()),
            };
            egui::ComboBox::from_id_salt("joint connected")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut j.connected, None, "World");
                    for (id, name) in targets {
                        ui.selectable_value(&mut j.connected, Some(*id), name);
                    }
                });
        });
        row(ui, "Anchor", |ui| vec3(ui, &mut j.anchor, 0.02, ""));
        if j.connected.is_some() {
            row(ui, "Connected Anchor", |ui| {
                vec3(ui, &mut j.connected_anchor, 0.02, "")
            });
        }
        row(ui, "Collide Connected", |ui| {
            ui.checkbox(&mut j.collide_connected, "");
        });
        match &mut j.kind {
            JointKind::Fixed | JointKind::Ball => {}
            JointKind::Hinge { axis, limits } => {
                row(ui, "Axis", |ui| vec3(ui, axis, 0.02, ""));
                limits_ui(ui, limits, "Angle Limits", "°", 1.0, -360.0..=360.0);
            }
            JointKind::Slider { axis, limits } => {
                row(ui, "Axis", |ui| vec3(ui, axis, 0.02, ""));
                limits_ui(ui, limits, "Limits", " m", 0.02, -1000.0..=1000.0);
            }
            JointKind::Spring {
                rest_length,
                stiffness,
                damping,
            } => {
                row(ui, "Rest Length", |ui| {
                    drag(ui, rest_length, 0.0..=1000.0, 0.02, " m")
                });
                row(ui, "Stiffness", |ui| {
                    drag(ui, stiffness, 0.0..=1.0e6, 1.0, "")
                });
                row(ui, "Damping", |ui| drag(ui, damping, 0.0..=1.0e5, 0.1, ""));
            }
        }
    });
}

/// An optional [min, max] range: a checkbox and the two ends.
fn limits_ui(
    ui: &mut egui::Ui,
    limits: &mut Option<[f32; 2]>,
    label: &str,
    suffix: &str,
    speed: f64,
    range: std::ops::RangeInclusive<f32>,
) {
    row(ui, label, |ui| {
        let mut on = limits.is_some();
        if ui.checkbox(&mut on, "").changed() {
            *limits = on.then_some([0.0, 1.0]);
        }
        if let Some([lo, hi]) = limits {
            drag(ui, lo, range.clone(), speed, suffix);
            drag(ui, hi, range, speed, suffix);
            if *hi < *lo {
                *hi = *lo;
            }
        }
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
                // The key is a map key: edit a copy that lives across frames
                // and apply it when the field loses focus.
                let edit_id = ui.make_persistent_id(("param-key", index, key.as_str()));
                let mut k = ui
                    .data_mut(|d| d.get_temp::<String>(edit_id))
                    .unwrap_or_else(|| key.clone());
                let response = ui.add(TextEdit::singleline(&mut k).desired_width(90.0));
                if response.has_focus() {
                    ui.data_mut(|d| d.insert_temp(edit_id, k.clone()));
                } else {
                    ui.data_mut(|d| d.remove_temp::<String>(edit_id));
                    if response.lost_focus() && k != *key {
                        rename = Some((key.clone(), k));
                    }
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
        && !script.params.contains_key(&new)
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

/// The Animation row of a Model: a clip dropdown (clips of the imported
/// model), or the graph's path / a summary of an inline graph with a clear button.
fn animation_combo(
    ui: &mut egui::Ui,
    animation: &mut Option<olvin_scene::file::AnimationDesc>,
    clips: &[String],
) {
    use olvin_scene::file::AnimationDesc;
    let mut clear = false;
    match animation {
        Some(AnimationDesc::Graph(path)) => {
            ui.text_edit_singleline(path);
            clear = ui.small_button("✖").on_hover_text("Clear").clicked();
        }
        Some(AnimationDesc::Inline(g)) => {
            ui.weak(format!("inline graph, {} states", g.states.len()));
            clear = ui.small_button("✖").on_hover_text("Clear").clicked();
        }
        _ if clips.is_empty() => match animation {
            Some(AnimationDesc::Clip(name)) => {
                ui.text_edit_singleline(name);
            }
            _ => {
                if ui.button("Add clip").clicked() {
                    *animation = Some(AnimationDesc::Clip(String::new()));
                }
            }
        },
        _ => {
            let current = match animation {
                Some(AnimationDesc::Clip(name)) if clips.contains(name) => name.clone(),
                Some(AnimationDesc::Clip(name)) => format!("{name} (missing)"),
                _ => "None".to_owned(),
            };
            egui::ComboBox::from_id_salt("model-clip")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(animation.is_none(), "None (rest pose)")
                        .clicked()
                    {
                        *animation = None;
                    }
                    for name in clips {
                        let on = matches!(animation, Some(AnimationDesc::Clip(c)) if c == name);
                        if ui.selectable_label(on, name).clicked() {
                            *animation = Some(AnimationDesc::Clip(name.clone()));
                        }
                    }
                });
        }
    }
    if clear {
        *animation = None;
    }
}

/// Which clip the scene view previews: the object's own setup, or any clip.
fn preview_clip_combo(
    ui: &mut egui::Ui,
    preview: &mut crate::engine::ModelPreview,
    clips: &[String],
) {
    egui::ComboBox::from_id_salt("model-preview-clip")
        .selected_text(preview.clip.as_deref().unwrap_or("Scene setup"))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut preview.clip, None, "Scene setup");
            for name in clips {
                ui.selectable_value(&mut preview.clip, Some(name.clone()), name);
            }
        });
}

/// A component block; `remove` gets `Some(key)` when its ✖ is clicked.
pub(crate) fn section(
    ui: &mut egui::Ui,
    title: &str,
    remove: Option<(&mut Option<&'static str>, &'static str)>,
    body: impl FnOnce(&mut egui::Ui),
) {
    // A prefab override of this block's field: blue title and a revert button.
    let overridden = remove
        .as_ref()
        .and_then(|(_, key)| field_of(key))
        .and_then(|field| Some((marks(ui).filter(|m| m.is_overridden(field))?, field)));
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
                let mut caption = theme::caps(ui.ctx(), title);
                if overridden.is_some() {
                    caption = caption.color(theme::palette(ui.ctx()).info);
                }
                ui.label(caption);
                if let Some((slot, key)) = remove {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("✖")
                            .on_hover_text("Remove component")
                            .clicked()
                        {
                            *slot = Some(key);
                        }
                        if let Some((marks, field)) = &overridden
                            && ui
                                .small_button("↺")
                                .on_hover_text("Revert to the prefab's value")
                                .clicked()
                        {
                            marks.revert(field);
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

/// Room left to the panel's edge for a widget at the cursor of a grid cell.
/// Not `available_width()`: a cell's room follows last frame's column width,
/// so it keeps any overflow (and an overflowing widget grows every section
/// below it). The clip rect is the visible panel; 26 px stay for the scroll bar, frames and
/// the padding of drag fields.
pub(crate) fn cell_width(ui: &egui::Ui) -> f32 {
    (ui.clip_rect().right() - 26.0 - ui.cursor().left()).max(0.0)
}

pub(crate) fn grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    // Labels take less room in a narrow panel so the fields keep theirs.
    let label_col = (cell_width(ui) * 0.3).clamp(52.0, 80.0);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([10.0, 4.0])
        .min_col_width(label_col)
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

/// Animation context of a vector property: Blender's field colours
/// (yellow = key on this frame, green = animated, orange = changed since the
/// last evaluation) and the right-click keyframe menu, `I` / `Alt+I` on hover.
struct Keying<'a> {
    action: &'a mut Action,
    object: u64,
    prop: Prop,
    frame: f32,
}

fn vec3_keyed(ui: &mut egui::Ui, v: &mut Vec3, speed: f64, suffix: &str, k: Keying) {
    let (width, letters) = vec3_fit(ui);
    let mut insert = false;
    let mut delete = false;
    let mut clear = false;
    let mut single = None;
    for (i, value) in [&mut v.x, &mut v.y, &mut v.z].into_iter().enumerate() {
        axis_letter(ui, i, letters);
        let state = k
            .action
            .key_state(k.object, k.prop, i as u8, k.frame, *value);
        let tint = match state {
            KeyState::None => None,
            KeyState::Keyed => Some((Color32::from_rgb(236, 205, 58), Color32::BLACK)),
            KeyState::Animated => Some((Color32::from_rgb(112, 178, 66), Color32::BLACK)),
            KeyState::Changed => Some((Color32::from_rgb(222, 128, 38), Color32::BLACK)),
        };
        let response = ui
            .scope(|ui| {
                if let Some((fill, text)) = tint {
                    let w = &mut ui.visuals_mut().widgets;
                    for style in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                        style.weak_bg_fill = fill;
                        style.bg_fill = fill;
                        style.fg_stroke.color = text;
                    }
                    ui.visuals_mut().override_text_color = Some(text);
                }
                ui.add_sized(
                    [width, 18.0],
                    DragValue::new(value)
                        .speed(speed)
                        .max_decimals(3)
                        .suffix(suffix),
                )
            })
            .inner;
        if response.hovered() && !ui.ctx().egui_wants_keyboard_input() {
            ui.input(|input| {
                if input.key_pressed(egui::Key::I) && input.modifiers.is_none() {
                    insert = true;
                }
                if input.key_pressed(egui::Key::I)
                    && input.modifiers.matches_exact(egui::Modifiers::ALT)
                {
                    delete = true;
                }
            });
        }
        response.context_menu(|ui| {
            if ui.button("Insert Keyframes  (I)").clicked() {
                insert = true;
                ui.close();
            }
            if ui.button("Insert Single Keyframe").clicked() {
                single = Some(i);
                ui.close();
            }
            let on_key = state == KeyState::Keyed;
            if ui
                .add_enabled(on_key, egui::Button::new("Delete Keyframes  (Alt+I)"))
                .clicked()
            {
                delete = true;
                ui.close();
            }
            if ui
                .add_enabled(
                    state != KeyState::None,
                    egui::Button::new("Clear Keyframes"),
                )
                .clicked()
            {
                clear = true;
                ui.close();
            }
        });
    }
    let Keying {
        action,
        object,
        prop,
        frame,
    } = k;
    if insert {
        action.insert_vec3(object, prop, frame, *v);
    }
    if let Some(i) = single {
        action.insert_key(object, prop, i as u8, frame, v[i]);
    }
    if delete {
        action.delete_keys_at(object, Some(prop), frame);
    }
    if clear {
        action
            .curves
            .retain(|c| !(c.object == object && c.prop == prop));
    }
}

/// Width of one field of a Vector3 row, whether the X/Y/Z letters fit, and
/// tighter spacing when the panel is narrow. The fields (and three letters,
/// five gaps) must fit the cell, or the whole section grows past the panel's
/// edge. A drag field is never narrower than ~34 px, so a very narrow panel
/// drops the letters.
fn vec3_fit(ui: &mut egui::Ui) -> (f32, bool) {
    const FIELD_MIN: f32 = 34.0;
    let wide = ui.spacing().item_spacing.x;
    let room = cell_width(ui);
    let narrow = (room - 3.0 * 11.0 - 5.0 * wide) / 3.0 < 40.0;
    // A narrow panel: tighter gaps, and fields may be thinner than a button.
    let gap = if narrow { 3.0 } else { wide };
    ui.spacing_mut().item_spacing.x = gap;
    let letters = (room - 3.0 * 11.0 - 5.0 * gap) / 3.0 >= FIELD_MIN;
    let width = if letters {
        (room - 3.0 * 11.0 - 5.0 * gap) / 3.0
    } else {
        (room - 2.0 * gap) / 3.0
    }
    .clamp(24.0, 90.0);
    ui.spacing_mut().interact_size.x = width.min(ui.spacing().interact_size.x);
    (width, letters)
}

/// The axis letter of a Vector3 row, or nothing in the compact layout.
fn axis_letter(ui: &mut egui::Ui, i: usize, letters: bool) {
    if letters {
        ui.label(
            RichText::new(["X", "Y", "Z"][i])
                .color(theme::palette(ui.ctx()).axes()[i])
                .strong(),
        );
    }
}

fn vec3(ui: &mut egui::Ui, v: &mut Vec3, speed: f64, suffix: &str) {
    let (width, letters) = vec3_fit(ui);
    for (i, value) in [&mut v.x, &mut v.y, &mut v.z].into_iter().enumerate() {
        axis_letter(ui, i, letters);
        ui.add_sized(
            [width, 18.0],
            DragValue::new(value)
                .speed(speed)
                .max_decimals(3)
                .suffix(suffix),
        );
    }
}

/// A terrain's size and texture layers (the brushes are scene view tools).
fn terrain_ui(ui: &mut egui::Ui, t: &mut olvin::scene::file::TerrainDesc, textures: &[String]) {
    use olvin::terrain::look::MAX_LAYERS;
    grid(ui, "terrain", |ui| {
        row(ui, "Data", |ui| {
            ui.add(egui::Label::new(RichText::new(&t.data).weak()).truncate())
                .on_hover_text("Heights and layer weights, written after each brush stroke");
        });
        row(ui, "Width", |ui| {
            drag(ui, &mut t.size.x, 1.0..=10000.0, 0.5, " m")
        });
        row(ui, "Length", |ui| {
            drag(ui, &mut t.size.z, 1.0..=10000.0, 0.5, " m")
        });
        row(ui, "Height", |ui| {
            drag(ui, &mut t.size.y, 0.0..=2000.0, 0.1, " m")
        });
    });
    ui.add_space(4.0);
    ui.label(RichText::new("Layers").strong());
    if t.layers.is_empty() {
        ui.weak("No layers: the terrain shows layer 1's stand-in colour (grass).");
    }
    let mut drop = None;
    for (i, layer) in t.layers.iter_mut().enumerate() {
        ui.push_id(("terrain-layer", i), |ui| {
            grid(ui, "layer", |ui| {
                row(ui, &format!("Layer {}", i + 1), |ui| {
                    let mut texture = (!layer.texture.is_empty()).then(|| layer.texture.clone());
                    if crate::materials::asset_combo(ui, "texture", &mut texture, textures, 28.0) {
                        layer.texture = texture.unwrap_or_default();
                    }
                    if ui
                        .small_button("✕")
                        .on_hover_text("Remove the layer")
                        .clicked()
                    {
                        drop = Some(i);
                    }
                });
                row(ui, "Tiling", |ui| {
                    drag(ui, &mut layer.tiling, 0.1..=1000.0, 0.1, "×")
                });
                row(ui, "Roughness", |ui| {
                    drag(ui, &mut layer.roughness, 0.0..=1.0, 0.01, "")
                });
            });
        });
    }
    if let Some(i) = drop {
        t.layers.remove(i);
    }
    if t.layers.len() < MAX_LAYERS && ui.button("+ Add Layer").clicked() {
        t.layers.push(Default::default());
    }
    ui.weak("Sculpt and paint with the Terrain tools in the scene view toolbar.");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Vector3 row in a grid inside a panel `width` wide: its right edge.
    fn right_edge(width: f32, keyed: bool) -> f32 {
        let ctx = egui::Context::default();
        let mut edge = 0.0;
        let mut action = Action::default();
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, 200.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    ui.set_clip_rect(rect);
                    let mut v = Vec3::new(-12.5, 20.0, 0.0);
                    grid(ui, "t", |ui| {
                        row(ui, "Rotation", |ui| {
                            if keyed {
                                let k = Keying {
                                    action: &mut action,
                                    object: 1,
                                    prop: Prop::Rotation,
                                    frame: 1.0,
                                };
                                vec3_keyed(ui, &mut v, 0.5, "°", k);
                            } else {
                                vec3(ui, &mut v, 0.5, "°");
                            }
                        });
                    });
                    edge = ui.min_rect().right() - rect.left();
                });
            });
            out.textures_delta.clear();
        }
        edge
    }

    #[test]
    fn vector3_rows_fit_any_panel_width() {
        for keyed in [false, true] {
            for width in [240.0, 260.0, 280.0, 310.0, 400.0, 600.0] {
                let edge = right_edge(width, keyed);
                assert!(
                    edge <= width + 0.5,
                    "keyed={keyed}: a {width} px panel got a {edge} px row"
                );
            }
        }
    }
}
