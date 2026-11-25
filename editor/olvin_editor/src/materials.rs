//! Material assets in the editor: finding `*.mat.ron`, textures and shaders
//! under `assets/`, and the material editor shown in the inspector.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use egui::{DragValue, RichText, Slider};
use olvin_scene::material::{MaterialDesc, SURFACE_SHADER_CONTRACT};

use crate::panels::inspector::{cell_width, grid, row};

const RESCAN: Duration = Duration::from_secs(2);
const IMAGE_EXT: [&str; 8] = ["png", "jpg", "jpeg", "tga", "bmp", "webp", "hdr", "exr"];

/// Content of the materials edited in the editor, by path: what an undo step
/// remembers (the files on disk always hold the current content).
pub type MaterialEdits = BTreeMap<String, MaterialDesc>;

/// What the project's `assets/` offers, rescanned every few seconds, plus the
/// loaded material files (reloaded when changed on disk).
#[derive(Default)]
pub struct MaterialLibrary {
    root: Option<PathBuf>,
    scanned: Option<Instant>,
    pub materials: Vec<String>,
    pub textures: Vec<String>,
    pub shaders: Vec<String>,
    /// `*.look.ron` picture looks.
    pub looks: Vec<String>,
    loaded: HashMap<String, (MaterialDesc, Option<SystemTime>)>,
    /// Current content of every material saved by the editor this session.
    edits: MaterialEdits,
    /// Content of those materials before their first edit (undo past it).
    originals: MaterialEdits,
}

impl MaterialLibrary {
    /// Rescans when the assets folder changed or the cache is old.
    pub fn refresh(&mut self, root: Option<&Path>) {
        let root = root.map(Path::to_path_buf);
        let fresh = self.scanned.is_some_and(|t| t.elapsed() < RESCAN);
        if root == self.root && fresh {
            return;
        }
        self.root = root;
        self.scanned = Some(Instant::now());
        self.materials.clear();
        self.textures.clear();
        self.shaders.clear();
        self.looks.clear();
        if let Some(root) = self.root.clone() {
            walk(&root, &root, &mut |rel| {
                let lower = rel.to_lowercase();
                if lower.ends_with(".mat.ron") {
                    self.materials.push(rel);
                } else if lower.ends_with(".look.ron") {
                    self.looks.push(rel);
                } else if lower.ends_with(".wgsl") {
                    self.shaders.push(rel);
                } else if IMAGE_EXT.iter().any(|e| lower.ends_with(&format!(".{e}"))) {
                    self.textures.push(rel);
                }
            });
        }
        for list in [
            &mut self.materials,
            &mut self.textures,
            &mut self.shaders,
            &mut self.looks,
        ] {
            list.sort();
        }
    }

    /// The material at `rel`, loaded (or reloaded after an outside change).
    pub fn get(&mut self, rel: &str) -> Option<&mut MaterialDesc> {
        let path = self.root.as_ref()?.join(rel);
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let stale = self.loaded.get(rel).is_none_or(|(_, t)| *t != mtime);
        if stale {
            let desc = MaterialDesc::load(&path).ok()?;
            if !self.edits.contains_key(rel) {
                self.originals.insert(rel.to_owned(), desc.clone());
            }
            self.loaded.insert(rel.to_owned(), (desc, mtime));
        }
        self.loaded.get_mut(rel).map(|(d, _)| d)
    }

    /// Writes an edited material and remembers its new mtime.
    pub fn save(&mut self, rel: &str) -> Result<(), String> {
        let root = self.root.clone().ok_or("no project")?;
        let path = root.join(rel);
        let (desc, mtime) = self.loaded.get_mut(rel).ok_or("not loaded")?;
        desc.save(&path)?;
        *mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.edits.insert(rel.to_owned(), desc.clone());
        Ok(())
    }

    /// Forgets cached content and edits of the materials `gone` matches.
    pub fn forget(&mut self, gone: &dyn Fn(&String) -> bool) {
        self.loaded.retain(|k, _| !gone(k));
        self.edits.retain(|k, _| !gone(k));
        self.originals.retain(|k, _| !gone(k));
        self.scanned = None;
    }

    /// The edited materials, for the undo history.
    pub fn edits(&self) -> &MaterialEdits {
        &self.edits
    }

    /// Rewrites the materials that differ from `target` (an undo step's
    /// edits); one that was not edited yet goes back to its original content.
    pub fn restore(&mut self, target: &MaterialEdits) -> Result<(), String> {
        let root = self.root.clone().ok_or("no project")?;
        let keys: Vec<String> = self.edits.keys().chain(target.keys()).cloned().collect();
        let mut result = Ok(());
        for rel in keys {
            let Some(want) = target.get(&rel).or_else(|| self.originals.get(&rel)) else {
                continue;
            };
            let current = self.edits.get(&rel).or_else(|| self.originals.get(&rel));
            if current != Some(want) {
                let path = root.join(&rel);
                match want.save(&path) {
                    Ok(()) => {
                        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                        self.loaded.insert(rel.clone(), (want.clone(), mtime));
                    }
                    Err(e) => {
                        result = Err(e);
                        continue;
                    }
                }
            }
            // The edit set mirrors the step: an unedited material is not in it.
            if target.contains_key(&rel) {
                self.edits.insert(rel, want.clone());
            } else {
                self.edits.remove(&rel);
            }
        }
        result
    }

    /// Creates `materials/<name>.mat.ron` (unique name) and returns its path.
    pub fn create(&mut self, name: &str, desc: MaterialDesc) -> Result<String, String> {
        let root = self.root.clone().ok_or("open a project first")?;
        let base: String = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let base = if base.trim_matches('_').is_empty() {
            "material".to_owned()
        } else {
            base
        };
        let mut rel = format!("materials/{base}.mat.ron");
        for n in 2.. {
            if !root.join(&rel).exists() {
                break;
            }
            rel = format!("materials/{base}_{n}.mat.ron");
        }
        desc.save(root.join(&rel))?;
        self.scanned = None;
        Ok(rel)
    }

    /// Creates `shaders/<name>.wgsl` from the surface shader template.
    pub fn create_shader(&mut self, name: &str) -> Result<String, String> {
        let root = self.root.clone().ok_or("open a project first")?;
        let rel = format!("shaders/{name}.wgsl");
        let path = root.join(&rel);
        if !path.exists() {
            let dir = path.parent().ok_or("shader path has no parent")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(&path, SURFACE_SHADER_CONTRACT).map_err(|e| e.to_string())?;
        }
        self.scanned = None;
        Ok(rel)
    }
}

fn walk(root: &Path, dir: &Path, found: &mut impl FnMut(String)) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, found);
        } else if let Ok(rel) = path.strip_prefix(root) {
            found(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Picker for an optional asset path from `options`.
pub(crate) fn asset_combo(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut Option<String>,
    options: &[String],
    // Room taken by the widgets before it in the row.
    reserve: f32,
) -> bool {
    let before = value.clone();
    let shown = value.as_deref().map_or("None".to_owned(), short_name);
    // Shrinks with the panel (the arrow and padding take ~30 px).
    let width = (cell_width(ui) - reserve - 30.0).clamp(40.0, 170.0);
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "None");
            for o in options {
                ui.selectable_value(value, Some(o.clone()), o.as_str());
            }
        });
    *value != before
}

pub fn short_name(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.trim_end_matches(".mat.ron").to_owned()
}

/// A 0..1 slider whose track shrinks so the value box stays in the cell.
fn unit_slider(ui: &mut egui::Ui, value: &mut f32) {
    ui.spacing_mut().slider_width = (cell_width(ui) - 62.0).clamp(24.0, 100.0);
    ui.add(Slider::new(value, 0.0..=1.0));
}

/// The material editor: every field of a [`MaterialDesc`]. Returns true when
/// something changed (the caller saves).
pub fn material_ui(
    ui: &mut egui::Ui,
    m: &mut MaterialDesc,
    textures: &[String],
    shaders: &[String],
) -> bool {
    let before = m.clone();
    grid(ui, "material", |ui| {
        row(ui, "Base Color", |ui| {
            crate::widgets::color_rgba(ui, &mut m.base_color);
            asset_combo(ui, "mat-base", &mut m.base_texture, textures, 36.0);
        });
        row(ui, "Metallic", |ui| {
            unit_slider(ui, &mut m.metallic);
        });
        row(ui, "Roughness", |ui| {
            unit_slider(ui, &mut m.roughness);
        });
        row(ui, "Normal Map", |ui| {
            asset_combo(ui, "mat-normal", &mut m.normal_texture, textures, 56.0);
            if m.normal_texture.is_some() {
                ui.add(
                    DragValue::new(&mut m.normal_strength)
                        .range(0.0..=4.0)
                        .speed(0.01),
                );
            }
        });
        row(ui, "Occl/Rough/Metal", |ui| {
            asset_combo(ui, "mat-orm", &mut m.orm_texture, textures, 0.0);
        });
        row(ui, "Emission", |ui| {
            crate::widgets::color_rgb(ui, &mut m.emissive);
            asset_combo(ui, "mat-emissive", &mut m.emissive_texture, textures, 36.0);
        });
        row(ui, "Tiling", |ui| {
            ui.add(DragValue::new(&mut m.uv_scale[0]).speed(0.01).prefix("x "));
            ui.add(DragValue::new(&mut m.uv_scale[1]).speed(0.01).prefix("y "));
        });
        row(ui, "Offset", |ui| {
            ui.add(DragValue::new(&mut m.uv_offset[0]).speed(0.01).prefix("x "));
            ui.add(DragValue::new(&mut m.uv_offset[1]).speed(0.01).prefix("y "));
        });
        row(ui, "Alpha Cutoff", |ui| {
            unit_slider(ui, &mut m.alpha_cutoff);
        });
        row(ui, "Triplanar", |ui| {
            ui.checkbox(&mut m.triplanar, "")
                .on_hover_text("World-space projection, no UVs needed (CSG levels)");
        });
        row(ui, "Pixelated", |ui| {
            ui.checkbox(&mut m.point_filter, "")
                .on_hover_text("Nearest filtering, no mipmaps");
        });
        row(ui, "Retro", |ui| {
            let mut state = m.retro;
            egui::ComboBox::from_id_salt("mat-retro")
                .selected_text(match state {
                    None => "Follow scene",
                    Some(true) => "On",
                    Some(false) => "Off",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state, None, "Follow scene");
                    ui.selectable_value(&mut state, Some(true), "On");
                    ui.selectable_value(&mut state, Some(false), "Off");
                })
                .response
                .on_hover_text(
                    "PS1 style for this material: snapping, affine UVs, vertex lighting",
                );
            m.retro = state;
        });
        row(ui, "Unlit", |ui| {
            ui.checkbox(&mut m.unlit, "");
        });
        row(ui, "Transparent", |ui| {
            ui.checkbox(&mut m.alpha_blend, "").on_hover_text(
                "Alpha blending: the colour's alpha is the opacity (sorted, casts no shadow)",
            );
        });
        row(ui, "Shader", |ui| {
            asset_combo(ui, "mat-shader", &mut m.shader, shaders, 0.0);
        });
    });
    *m != before
}

type Preset = (&'static str, fn(&mut MaterialDesc));

/// Small preset row for fast starting points.
pub fn presets_ui(ui: &mut egui::Ui, m: &mut MaterialDesc) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Presets").weak());
        let presets: [Preset; 5] = [
            ("Plastic", |m| (m.metallic, m.roughness) = (0.0, 0.45)),
            ("Metal", |m| (m.metallic, m.roughness) = (1.0, 0.3)),
            ("Rough", |m| (m.metallic, m.roughness) = (0.0, 0.95)),
            ("Mirror", |m| (m.metallic, m.roughness) = (1.0, 0.02)),
            ("Glow", |m| {
                m.emissive = [
                    m.base_color[0] * 2.0,
                    m.base_color[1] * 2.0,
                    m.base_color[2] * 2.0,
                ]
            }),
        ];
        for (name, apply) in presets {
            if ui.small_button(name).clicked() {
                apply(m);
                changed = true;
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_edit_save_and_reload_from_disk() {
        let root = std::env::temp_dir().join(format!("olvin-mat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("textures")).unwrap();
        std::fs::write(root.join("textures/brick.png"), b"not really a png").unwrap();
        let mut lib = MaterialLibrary::default();
        lib.refresh(Some(&root));
        assert_eq!(lib.textures, ["textures/brick.png"]);

        let rel = lib.create("Red Brick", MaterialDesc::default()).unwrap();
        assert_eq!(rel, "materials/Red_Brick.mat.ron");
        assert_eq!(
            lib.create("Red Brick", MaterialDesc::default()).unwrap(),
            "materials/Red_Brick_2.mat.ron"
        );
        lib.get(&rel).unwrap().base_texture = Some("textures/brick.png".into());
        lib.save(&rel).unwrap();
        assert_eq!(
            MaterialDesc::load(root.join(&rel))
                .unwrap()
                .base_texture
                .as_deref(),
            Some("textures/brick.png")
        );

        // An outside edit (e.g. the IDE) is picked up.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut outside = MaterialDesc::load(root.join(&rel)).unwrap();
        outside.metallic = 1.0;
        outside.save(root.join(&rel)).unwrap();
        let mtime = std::time::SystemTime::now() + std::time::Duration::from_secs(1);
        let f = std::fs::File::options()
            .write(true)
            .open(root.join(&rel))
            .unwrap();
        f.set_modified(mtime).unwrap();
        assert_eq!(lib.get(&rel).unwrap().metallic, 1.0);

        let shader = lib.create_shader("glow").unwrap();
        assert!(
            std::fs::read_to_string(root.join(shader))
                .unwrap()
                .contains("fn surface(")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
