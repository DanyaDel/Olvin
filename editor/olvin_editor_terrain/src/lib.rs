//! Editor element: terrain brushes in the scene view.
//!
//! Raise (Shift: lower), Smooth, Flatten (to the height under the stroke's
//! start) and Paint (a texture layer). The brush works on the selected
//! terrain, or on the one under the pointer; a stroke is one undo step and
//! writes the terrain's file when it ends (see `EditorState::terrain_stroke_*`).

use olvin_editor::egui::{self, Color32, Pos2, Slider};
use olvin_editor::ext::{Ray, ToolDesc, ViewportCtx, ViewportExtension};
use olvin_editor::glam::{Mat4, Vec2, Vec3};
use olvin_editor::olvin::terrain::brush::{self, Brush, BrushOp};
use olvin_editor::olvin::terrain::{TerrainDesc, mesh};
use olvin_editor::{EditorPlugin, EditorState, Registry};

pub const RAISE: &str = "terrain.raise";
pub const SMOOTH: &str = "terrain.smooth";
pub const FLATTEN: &str = "terrain.flatten";
pub const PAINT: &str = "terrain.paint";

const TOOLS: [ToolDesc; 4] = [
    ToolDesc {
        id: RAISE,
        label: "Raise",
        tooltip: "Terrain: raise under the brush (Shift: lower)",
        key: None,
    },
    ToolDesc {
        id: SMOOTH,
        label: "Smooth",
        tooltip: "Terrain: even out bumps under the brush",
        key: None,
    },
    ToolDesc {
        id: FLATTEN,
        label: "Flatten",
        tooltip: "Terrain: level to the height where the stroke started",
        key: None,
    },
    ToolDesc {
        id: PAINT,
        label: "Paint",
        tooltip: "Terrain: paint the chosen texture layer",
        key: None,
    },
];

/// Terrain heights per second at full strength (a full-height hill in four).
const RAISE_RATE: f32 = 0.25;

pub struct TerrainPlugin;

impl EditorPlugin for TerrainPlugin {
    fn name(&self) -> &'static str {
        "Terrain"
    }

    fn build(&self, registry: &mut Registry) {
        registry.add_viewport_extension(|| Box::<TerrainTools>::default());
    }
}

/// Brush settings, kept in egui's memory (they outlive the tool switch).
#[derive(Clone, Copy, Debug)]
struct Settings {
    radius: f32,
    strength: f32,
    layer: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            radius: 5.0,
            strength: 0.5,
            layer: 1,
        }
    }
}

#[derive(Default)]
pub struct TerrainTools {
    /// The terrain a stroke is on: (object id, Flatten's target height).
    stroke: Option<(u64, f32)>,
}

/// A terrain under the pointer: its object, description, world matrix and
/// the hit in its local space.
struct Target {
    id: u64,
    desc: TerrainDesc,
    world: Mat4,
    local: Vec3,
}

/// The terrain the ray meets first: the selected one if it is hit, else the
/// nearest (or only `only`, during a stroke).
fn target(state: &mut EditorState, ray: &Ray, only: Option<u64>) -> Option<Target> {
    let worlds = state.scene.world_matrices();
    let root = state.assets_dir();
    let candidates: Vec<(u64, TerrainDesc, Mat4)> = state
        .scene
        .entities
        .iter()
        .filter(|e| state.scene.active_in_hierarchy(e.id))
        .filter(|e| only.is_none_or(|o| o == e.id))
        .filter_map(|e| Some((e.id, e.terrain.clone()?, *worlds.get(&e.id)?)))
        .collect();
    let selected = state.selection;
    let mut best: Option<(bool, f32, Target)> = None;
    for (id, desc, world) in candidates {
        let inv = world.inverse();
        let (o, d) = (
            inv.transform_point3(ray.origin),
            inv.transform_vector3(ray.dir),
        );
        let data = &state.terrains.entry(root.as_deref(), &desc.data).data;
        let Some(local) = mesh::raycast(data, desc.size, o, d) else {
            continue;
        };
        let distance = world.transform_point3(local).distance(ray.origin);
        let preferred = selected == Some(id);
        let better = best
            .as_ref()
            .is_none_or(|(p, dist, _)| (preferred && !p) || (preferred == *p && distance < *dist));
        if better {
            best = Some((
                preferred,
                distance,
                Target {
                    id,
                    desc,
                    world,
                    local,
                },
            ));
        }
    }
    best.map(|(_, _, t)| t)
}

impl ViewportExtension for TerrainTools {
    fn geometry(&mut self, _: &EditorState, _: &mut olvin_editor::ext::Geometry) {}

    fn tools(&self) -> &[ToolDesc] {
        &TOOLS
    }

    fn interact(&mut self, ctx: &mut ViewportCtx) -> bool {
        if ctx.state.is_playing() || !TOOLS.iter().any(|t| t.id == ctx.tool) {
            self.stroke = None;
            ctx.state.terrain_stroke_end();
            return false;
        }
        if !ctx.state.terrain_stroking() {
            self.stroke = None;
        }
        let ui = ctx.ui;
        let id = egui::Id::new("olvin-terrain-brush");
        let mut settings = ui.data(|d| d.get_temp::<Settings>(id)).unwrap_or_default();
        settings_window(ui, ctx, &mut settings);
        ui.data_mut(|d| d.insert_temp(id, settings));

        let (down, pressed, shift, alt, dt) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.primary_pressed(),
                i.modifiers.shift,
                i.modifiers.alt,
                i.stable_dt.min(0.1),
            )
        });
        let pointer = ctx.response.hover_pos();
        let only = self.stroke.map(|(id, _)| id);
        let hit = pointer.and_then(|p| target(ctx.state, &ctx.camera.ray(p), only));

        if let Some(t) = &hit {
            brush_circle(ctx, t, settings.radius);
        }
        // Releasing (or leaving the view) ends the stroke: one undo step.
        if !down || alt || pointer.is_none() {
            if self.stroke.take().is_some() {
                ctx.state.terrain_stroke_end();
            }
            return hit.is_some();
        }
        let Some(t) = hit else {
            return self.stroke.is_some();
        };
        if self.stroke.is_none() && (!pressed || !ctx.response.is_pointer_button_down_on()) {
            return true;
        }
        let rel = t.desc.data.clone();
        if self.stroke.is_none() {
            ctx.state.terrain_stroke_begin(&rel);
            let start = (t.local.y / t.desc.size.y.max(1e-4)).clamp(0.0, 1.0);
            self.stroke = Some((t.id, start));
        }
        let flatten_to = self.stroke.map_or(0.0, |(_, h)| h);
        let op = match ctx.tool {
            RAISE if shift => BrushOp::Raise(-RAISE_RATE),
            RAISE => BrushOp::Raise(RAISE_RATE),
            SMOOTH => BrushOp::Smooth,
            FLATTEN => BrushOp::Flatten(flatten_to),
            _ => BrushOp::Paint(settings.layer.min(t.desc.layers.len().max(1) - 1)),
        };
        let b = Brush {
            center: Vec2::new(t.local.x, t.local.z),
            radius: settings.radius,
            strength: settings.strength,
        };
        let root = ctx.state.assets_dir();
        let data = &mut ctx.state.terrains.entry(root.as_deref(), &rel).data;
        if let Some(rect) = brush::apply(data, t.desc.size, &b, op, dt) {
            if matches!(op, BrushOp::Paint(_)) {
                ctx.state.terrains.weights_changed(&rel);
            } else {
                ctx.state.terrains.heights_changed(&rel, rect);
            }
        }
        ui.ctx().request_repaint();
        true
    }
}

/// The brush's outline lying on the surface.
fn brush_circle(ctx: &ViewportCtx, t: &Target, radius: f32) {
    let steps = 48;
    let mut points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        let at = Vec2::new(t.local.x, t.local.z) + Vec2::new(a.cos(), a.sin()) * radius;
        let y = ground(ctx, t, at).unwrap_or(t.local.y);
        points.push(t.world.transform_point3(Vec3::new(at.x, y + 0.05, at.y)));
    }
    for w in points.windows(2) {
        ctx.camera.line(
            ctx.painter,
            w[0],
            w[1],
            (2.0, Color32::from_rgb(255, 200, 80)),
        );
    }
}

/// Surface height at local `at`, from the cached data (read only).
fn ground(ctx: &ViewportCtx, t: &Target, at: Vec2) -> Option<f32> {
    let data = ctx.state.terrains.peek(&t.desc.data)?;
    mesh::height_at(data, t.desc.size, at)
}

/// Brush settings in the scene view's corner while a terrain tool is active.
fn settings_window(ui: &egui::Ui, ctx: &ViewportCtx, s: &mut Settings) {
    let at = Pos2::new(ctx.response.rect.left() + 10.0, ctx.overlay_top + 8.0);
    egui::Area::new(egui::Id::new("olvin-terrain-brush-window"))
        .fixed_pos(at)
        .order(egui::Order::Foreground)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(220.0);
                ui.strong("▲ Terrain brush");
                ui.add(
                    Slider::new(&mut s.radius, 0.5..=100.0)
                        .logarithmic(true)
                        .text("Radius (m)"),
                );
                ui.add(Slider::new(&mut s.strength, 0.01..=1.0).text("Strength"));
                if ctx.tool == PAINT {
                    ui.horizontal(|ui| {
                        ui.label("Layer");
                        for i in 0..4 {
                            ui.selectable_value(&mut s.layer, i, format!("{}", i + 1));
                        }
                    });
                }
                ui.weak(match ctx.tool {
                    RAISE => "LMB raise · Shift+LMB lower",
                    SMOOTH => "LMB smooth",
                    FLATTEN => "LMB: level to where the stroke starts",
                    _ => "LMB paint the layer (set its texture in the Inspector)",
                });
            });
        });
}
