//! Editor element: the Library panel. Browse the built-in preset packs
//! (plus `$OLVIN_LIBRARY` and `<project>/library/`), import them into the
//! project, add a pack's headline prefab to the scene.
//!
//! A panel of its own element rather than part of `olvin_editor`: it is
//! optional (hub catalog id `library`), needs nothing the plugin API does not
//! give (panels get `EditorState`), and keeps the `olvin_library` crate and its
//! embedded packs out of editors installed without it.

use std::collections::HashMap;

use olvin_editor::egui::{self, Color32, RichText, TextEdit};
use olvin_editor::glam::Vec3;
use olvin_editor::theme::{self, Palette};
use olvin_editor::{EditorPlugin, EditorState, Panel, PanelKind, Registry};
use olvin_library::{
    Category, ImportOptions, ImportReport, Library, OnConflict, Pack, Source, Status,
};

pub struct LibraryPlugin;

impl EditorPlugin for LibraryPlugin {
    fn name(&self) -> &'static str {
        "Library"
    }

    fn build(&self, registry: &mut Registry) {
        registry.add_panel(PanelKind {
            id: "library.browser",
            title: "📦 Library",
            factory: || Box::new(LibraryPanel::default()),
        });
    }
}

/// A message under the filters: the result of an import with its notes, or an error.
struct Toast {
    text: String,
    notes: Vec<(String, String)>,
    error: bool,
    /// Pack to offer "Open demo scene" for.
    demo: Option<String>,
}

/// An import waiting for the user's decision about files that differ.
struct Conflicts {
    pack: String,
    /// `(pack id, path)`.
    files: Vec<(String, String)>,
}

#[derive(Default)]
struct LibraryPanel {
    library: Option<Library>,
    /// The project the library was loaded for (its `library/` folder adds packs).
    loaded_for: Option<std::path::PathBuf>,
    category: Option<Category>,
    query: String,
    toast: Option<Toast>,
    conflicts: Option<Conflicts>,
    previews: HashMap<String, Option<egui::TextureHandle>>,
}

impl Panel for LibraryPanel {
    fn ui(&mut self, ui: &mut egui::Ui, state: &mut EditorState) {
        let p = theme::palette(ui.ctx());
        if self.library.is_none() || self.loaded_for != state.project {
            self.reload(state);
        }
        let Some(library) = self.library.take() else {
            return;
        };

        self.filters(ui, &library, &p);
        if let Some(problem) = library.problems().first() {
            ui.colored_label(p.warn, format!("⚠ {problem}"))
                .on_hover_text(library.problems().join("\n"));
        }
        self.toast_ui(ui, state, &library, &p);

        let mut action = None;
        let packs = library.search(&self.query, self.category);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if packs.is_empty() {
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.colored_label(p.muted, "No packs match.");
                    });
                }
                for pack in packs {
                    let status = state
                        .project
                        .as_deref()
                        .map(|project| library.status(pack.id(), project));
                    if let Some(a) = self.card(ui, state, pack, status.as_ref(), &p) {
                        action = Some(a);
                    }
                    ui.add_space(6.0);
                }
            });
        self.library = Some(library);

        match action {
            Some(Action::Import(id)) => self.import(state, &id, None),
            Some(Action::AddToScene(id)) => self.add_to_scene(state, &id),
            Some(Action::OpenDemo(id)) => self.open_demo(state, &id),
            None => {}
        }
        self.conflict_dialog(ui.ctx(), state);
    }
}

enum Action {
    Import(String),
    AddToScene(String),
    OpenDemo(String),
}

impl LibraryPanel {
    fn reload(&mut self, state: &EditorState) {
        self.library = Some(Library::for_project(state.project.as_deref()));
        self.loaded_for = state.project.clone();
        self.previews.clear();
    }

    fn filters(&mut self, ui: &mut egui::Ui, library: &Library, p: &Palette) {
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.query)
                    .hint_text("🔍 Search packs")
                    .desired_width(ui.available_width() - 70.0),
            );
            if ui
                .button("⟲")
                .on_hover_text("Reload the library (user packs in <project>/library/)")
                .clicked()
            {
                self.library = None;
            }
        });
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            let total = library.list().len();
            let tab = |ui: &mut egui::Ui, selected: bool, text: String| {
                ui.selectable_label(
                    selected,
                    RichText::new(text).color(if selected { p.text } else { p.muted }),
                )
            };
            if tab(ui, self.category.is_none(), format!("All {total}")).clicked() {
                self.category = None;
            }
            for category in Category::ALL {
                let n = library.search("", Some(category)).len();
                if n == 0 && self.category != Some(category) {
                    continue;
                }
                if tab(
                    ui,
                    self.category == Some(category),
                    format!("{} {n}", category.label()),
                )
                .clicked()
                {
                    self.category = Some(category);
                }
            }
        });
        ui.separator();
    }

    fn toast_ui(
        &mut self,
        ui: &mut egui::Ui,
        state: &mut EditorState,
        library: &Library,
        p: &Palette,
    ) {
        let Some(toast) = &self.toast else {
            return;
        };
        let accent = if toast.error { p.error } else { p.accent };
        let mut close = false;
        let mut open_demo = None;
        egui::Frame::new()
            .fill(p.surface_raised)
            .stroke(egui::Stroke::new(1.0, accent))
            .corner_radius(theme::RADIUS_MD)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&toast.text).color(p.text)).wrap());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.small_button("✖").on_hover_text("Dismiss").clicked() {
                            close = true;
                        }
                    });
                });
                for (id, notes) in &toast.notes {
                    let name = library
                        .get(id)
                        .map_or(id.as_str(), |p| p.manifest.name.as_str());
                    ui.add_space(2.0);
                    ui.label(RichText::new(name).color(p.text_2).strong());
                    ui.add(egui::Label::new(RichText::new(notes).color(p.text_2)).wrap());
                }
                if let Some(id) = &toast.demo {
                    ui.add_space(4.0);
                    if ui.button("Open demo scene").clicked() {
                        open_demo = Some(id.clone());
                    }
                }
            });
        ui.add_space(6.0);
        if let Some(id) = open_demo {
            self.open_demo(state, &id);
        }
        if close {
            self.toast = None;
        }
    }

    fn card(
        &mut self,
        ui: &mut egui::Ui,
        state: &EditorState,
        pack: &Pack,
        status: Option<&Status>,
        p: &Palette,
    ) -> Option<Action> {
        let m = &pack.manifest;
        let installed = matches!(status, Some(Status::Installed(_) | Status::Outdated { .. }));
        let mut action = None;
        egui::Frame::new()
            .fill(p.surface_raised)
            .stroke(egui::Stroke::new(1.0, p.line_soft))
            .corner_radius(theme::RADIUS_MD)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    self.preview(ui, pack, p);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(&m.name)
                                    .color(p.text)
                                    .strong()
                                    .size(theme::FS_LG),
                            );
                            ui.label(
                                RichText::new(format!("v{}", m.version))
                                    .color(p.muted)
                                    .monospace(),
                            );
                            chip(ui, m.category.label(), p.muted, p);
                            match status {
                                Some(Status::Installed(_)) => badge(ui, "Installed", p.accent, p),
                                Some(Status::Outdated { installed, .. }) => {
                                    badge(ui, &format!("Installed {installed}"), p.warn, p)
                                }
                                _ => {}
                            }
                            if pack.source != Source::Builtin {
                                chip(ui, "user pack", p.info, p);
                            }
                        });
                        ui.add(
                            egui::Label::new(RichText::new(&m.description).color(p.text_2)).wrap(),
                        );
                        if !m.tags.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                for tag in &m.tags {
                                    chip(ui, &format!("#{tag}"), p.muted, p);
                                }
                            });
                        }
                    });
                });
                ui.add_space(4.0);
                if !m.requires.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Requires").color(p.muted));
                        for r in &m.requires {
                            ui.label(RichText::new(r).monospace().color(p.text_2));
                        }
                    });
                }
                if !m.provides.is_empty() {
                    egui::CollapsingHeader::new(
                        RichText::new(format!("Provides ({})", m.provides.len())).color(p.muted),
                    )
                    .id_salt(("provides", &m.id))
                    .show(ui, |ui| {
                        for path in &m.provides {
                            ui.label(
                                RichText::new(path)
                                    .monospace()
                                    .color(p.text_2)
                                    .size(theme::FS_SM),
                            );
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    let has_project = state.project.is_some();
                    let label = match status {
                        Some(Status::Installed(_)) => "Re-import",
                        Some(Status::Outdated { .. }) => "Update",
                        _ => "Import",
                    };
                    let import = egui::Button::new(RichText::new(label).color(if installed {
                        p.text
                    } else {
                        p.on_accent
                    }))
                    .fill(if installed { p.glass } else { p.accent });
                    if ui
                        .add_enabled(has_project, import)
                        .on_hover_text(if has_project {
                            "Copy the pack into the project's assets/"
                        } else {
                            "Open a project first"
                        })
                        .clicked()
                    {
                        action = Some(Action::Import(m.id.clone()));
                    }
                    if pack.headline_prefab().is_some()
                        && ui
                            .add_enabled(installed, egui::Button::new("Add to scene"))
                            .on_hover_text("Instantiate the pack's prefab in the open scene")
                            .on_disabled_hover_text("Import the pack first")
                            .clicked()
                    {
                        action = Some(Action::AddToScene(m.id.clone()));
                    }
                    if pack.demo_scene().is_some()
                        && ui
                            .add_enabled(installed, egui::Button::new("Open demo"))
                            .on_hover_text("Open the pack's demo scene, then press Play")
                            .on_disabled_hover_text("Import the pack first")
                            .clicked()
                    {
                        action = Some(Action::OpenDemo(m.id.clone()));
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} files, {} KB · {} · {}",
                            pack.files.len(),
                            pack.total_bytes().div_ceil(1024),
                            m.author,
                            m.license
                        ))
                        .color(p.muted)
                        .size(theme::FS_SM),
                    );
                });
            });
        action
    }

    /// `preview.png` as a texture, or a plain tile with the category's initial.
    fn preview(&mut self, ui: &mut egui::Ui, pack: &Pack, p: &Palette) {
        let size = egui::vec2(96.0, 64.0);
        let texture = self
            .previews
            .entry(pack.manifest.id.clone())
            .or_insert_with(|| {
                let bytes = pack.preview.as_ref()?;
                let img = image::load_from_memory(bytes).ok()?.to_rgba8();
                let (w, h) = (img.width() as usize, img.height() as usize);
                let color = egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw());
                Some(ui.ctx().load_texture(
                    format!("library-preview-{}", pack.manifest.id),
                    color,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .clone();
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let rounding = theme::RADIUS_SM;
        match texture {
            Some(t) => {
                egui::Image::new(&t)
                    .corner_radius(rounding)
                    .fit_to_exact_size(size)
                    .paint_at(ui, rect);
            }
            None => {
                ui.painter().rect_filled(rect, rounding, p.glass);
                ui.painter().rect_stroke(
                    rect,
                    rounding,
                    egui::Stroke::new(1.0, p.line_soft),
                    egui::StrokeKind::Inside,
                );
                let initial = pack.manifest.category.label().chars().next().unwrap_or('?');
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    initial,
                    egui::FontId::proportional(28.0),
                    p.muted,
                );
            }
        }
    }

    /// Plans first: files that differ ask the user; otherwise imports at once.
    fn import(&mut self, state: &mut EditorState, id: &str, policy: Option<OnConflict>) {
        let Some(project) = state.project.clone() else {
            return;
        };
        let Some(library) = self.library.as_ref() else {
            return;
        };
        if policy.is_none() {
            match library.plan(id, &project) {
                Ok(plan) => {
                    let files: Vec<(String, String)> = plan
                        .conflicts()
                        .into_iter()
                        .map(|(p, f)| (p.to_owned(), f.to_owned()))
                        .collect();
                    if !files.is_empty() {
                        self.conflicts = Some(Conflicts {
                            pack: id.to_owned(),
                            files,
                        });
                        return;
                    }
                }
                Err(e) => return self.fail(state, e.to_string()),
            }
        }
        let options = ImportOptions {
            on_conflict: policy.unwrap_or_default(),
        };
        match library.import(id, &project, &options) {
            Ok(report) => self.imported(state, id, &report),
            Err(e) => self.fail(state, e.to_string()),
        }
    }

    fn imported(&mut self, state: &mut EditorState, id: &str, report: &ImportReport) {
        let summary = report.summary();
        state.info(format!("Library: {summary}"));
        for w in &report.warnings {
            state.warn(format!("Library: {w}"));
        }
        // Show the new files in the Assets panel (and let it rescan).
        if let Some(pack) = self.library.as_ref().and_then(|l| l.get(id))
            && let Some(assets) = state.assets_dir()
        {
            let first = pack
                .manifest
                .provides
                .first()
                .or_else(|| pack.files.first().map(|f| &f.path));
            if let Some(rel) = first {
                state.asset_focus = Some(assets.join(rel));
            }
            // Materials, textures and shaders the Inspector offers.
            state.materials.refresh(None);
            state.materials.refresh(Some(&assets));
        }
        self.toast = Some(Toast {
            text: summary,
            notes: report.notes.clone(),
            error: false,
            demo: self
                .library
                .as_ref()
                .and_then(|l| l.get(id))
                .filter(|p| p.demo_scene().is_some())
                .map(|p| p.manifest.id.clone()),
        });
    }

    fn fail(&mut self, state: &mut EditorState, text: String) {
        state.error(format!("Library: {text}"));
        self.toast = Some(Toast {
            text,
            notes: Vec::new(),
            error: true,
            demo: None,
        });
    }

    fn add_to_scene(&mut self, state: &mut EditorState, id: &str) {
        let Some(rel) = self
            .library
            .as_ref()
            .and_then(|l| l.get(id))
            .and_then(Pack::headline_prefab)
            .map(str::to_owned)
        else {
            return;
        };
        // Characters stand on the ground: lift them by half their height.
        let lift = state
            .prefabs
            .get(&rel)
            .ok()
            .and_then(|scene| {
                scene
                    .entities
                    .iter()
                    .find(|e| e.parent.is_none())
                    .and_then(|e| {
                        e.character_controller
                            .as_ref()
                            .map(|c| c.height / 2.0 + 0.05)
                    })
            })
            .unwrap_or(0.0);
        match state.instantiate_prefab(&rel, None, Vec3::new(0.0, lift, 0.0)) {
            Ok(_) => {
                state.info(format!("Library: added {rel} to the scene"));
                self.toast = Some(Toast {
                    text: format!(
                        "Added {} to the scene",
                        rel.rsplit('/').next().unwrap_or(&rel)
                    ),
                    notes: Vec::new(),
                    error: false,
                    demo: None,
                });
            }
            Err(e) => self.fail(state, e),
        }
    }

    fn open_demo(&mut self, state: &mut EditorState, id: &str) {
        let Some(assets) = state.assets_dir() else {
            return;
        };
        let Some(rel) = self
            .library
            .as_ref()
            .and_then(|l| l.get(id))
            .and_then(Pack::demo_scene)
        else {
            return;
        };
        // Asks before dropping unsaved changes, like opening from the Assets panel.
        state.request_open(assets.join(rel));
    }

    fn conflict_dialog(&mut self, ctx: &egui::Context, state: &mut EditorState) {
        let Some(c) = &self.conflicts else {
            return;
        };
        let p = theme::palette(ctx);
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("library-conflicts")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Files already exist");
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} file(s) in the project differ from the pack's. Nothing is overwritten until you choose.",
                    c.files.len()
                ))
                .color(p.text_2),
            );
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for (pack, path) in &c.files {
                    ui.label(RichText::new(format!("{path}  ({pack})")).monospace().size(theme::FS_SM));
                }
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("Replace")
                    .on_hover_text("Overwrite the project's files with the pack's")
                    .clicked()
                {
                    choice = Some(Some(OnConflict::Replace));
                }
                if ui
                    .button("Keep both")
                    .on_hover_text("Write the pack's file next to it with a -2 suffix")
                    .clicked()
                {
                    choice = Some(Some(OnConflict::KeepBoth));
                }
                if ui
                    .button("Skip these")
                    .on_hover_text("Keep the project's files, import the rest")
                    .clicked()
                {
                    choice = Some(Some(OnConflict::Skip));
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(None);
                }
            });
        });
        if modal.should_close() && choice.is_none() {
            choice = Some(None);
        }
        if let Some(choice) = choice {
            let pack = self.conflicts.take().map(|c| c.pack).unwrap_or_default();
            if let Some(policy) = choice {
                self.import(state, &pack, Some(policy));
            }
        }
    }
}

fn tinted(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40)
}

/// A small outlined label: category, tag.
fn chip(ui: &mut egui::Ui, text: &str, color: Color32, p: &Palette) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, p.line_soft))
        .corner_radius(theme::RADIUS_PILL)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(theme::FS_XS));
        });
}

/// A filled label: Installed, Update.
fn badge(ui: &mut egui::Ui, text: &str, color: Color32, _p: &Palette) {
    egui::Frame::new()
        .fill(tinted(color))
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(theme::RADIUS_PILL)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("✔ {text}"))
                    .color(color)
                    .size(theme::FS_XS)
                    .strong(),
            );
        });
}
