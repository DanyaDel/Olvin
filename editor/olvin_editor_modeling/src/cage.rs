//! The Edit Mode cage for one frame: world and screen positions, cached
//! topology, occlusion by the mesh itself (X-ray off), drawing and picking.

use olvin_editor::egui::{self, Color32, Pos2, Rect, Stroke, vec2};
use olvin_editor::ext::ViewCamera;
use olvin_editor::glam::{Mat4, Vec3};
use olvin_editor::olvin_scene::file::PolyMesh;
use olvin_editor::theme;

use crate::select::{Occluders, box_edges};
use crate::{Mode, mask, shared};

pub(crate) const HOT: Color32 = Color32::from_rgb(255, 225, 70);
const WIRE: Color32 = Color32::from_rgb(20, 22, 26);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Hover {
    Vertex(u32),
    Edge(u32, u32, f32),
    Face(usize),
}

impl Hover {
    pub fn verts(self, mesh: &PolyMesh) -> Vec<u32> {
        match self {
            Hover::Vertex(v) => vec![v],
            Hover::Edge(a, b, _) => vec![a, b],
            Hover::Face(f) => mesh.faces[f].clone(),
        }
    }
}

/// Visibility inputs; the occlusion pass reruns only when they change.
type VisKey = (Mat4, Mat4, Vec<u32>, bool);

#[derive(Default)]
pub(crate) struct Cage {
    /// The mesh `edges` and `tris` were built from.
    mesh: PolyMesh,
    vis_key: Option<VisKey>,
    pub edges: Vec<[u32; 2]>,
    /// Triangles with their face.
    pub tris: Vec<(usize, [u32; 3])>,
    pub points: Vec<Vec3>,
    pub screen: Vec<Option<Pos2>>,
    pub centers: Vec<Vec3>,
    pub center_screen: Vec<Option<Pos2>>,
    pub selected: Vec<bool>,
    pub face_selected: Vec<bool>,
    pub hidden: Vec<bool>,
    pub face_hidden: Vec<bool>,
    /// Shown and not occluded (or X-ray): drawn bright and pickable.
    pub vis_v: Vec<bool>,
    pub vis_e: Vec<bool>,
    pub vis_f: Vec<bool>,
}

impl Cage {
    /// Refreshes everything for this frame. `frozen` (during modal
    /// operations) keeps last frame's visibility when it still fits.
    pub fn update(&mut self, mesh: &PolyMesh, world: Mat4, camera: &ViewCamera, frozen: bool) {
        let (verts, hidden, xray) = shared(|s| (s.verts.clone(), s.hidden.clone(), s.xray));
        let changed = self.mesh != *mesh;
        if changed {
            self.mesh.clone_from(mesh);
            self.edges = mesh.edges();
            self.tris.clear();
            for f in 0..mesh.faces.len() {
                self.tris
                    .extend(mesh.face_triangles(f).into_iter().map(|t| (f, t)));
            }
        }
        let n = mesh.positions.len();
        self.points.clear();
        self.points
            .extend(mesh.positions.iter().map(|p| world.transform_point3(*p)));
        self.screen.clear();
        self.screen
            .extend(self.points.iter().map(|p| camera.project(*p)));
        self.centers.clear();
        self.centers
            .extend((0..mesh.faces.len()).map(|f| world.transform_point3(mesh.face_center(f))));
        self.center_screen.clear();
        self.center_screen
            .extend(self.centers.iter().map(|p| camera.project(*p)));
        self.selected = mask(n, &verts);
        self.hidden = mask(n, &hidden);
        self.face_selected = mesh
            .faces
            .iter()
            .map(|f| f.iter().all(|v| self.selected[*v as usize]))
            .collect();
        self.face_hidden = mesh
            .faces
            .iter()
            .map(|f| f.iter().any(|v| self.hidden[*v as usize]))
            .collect();

        let key = (camera.view_proj, world, hidden, xray);
        let fits = self.vis_v.len() == n
            && self.vis_e.len() == self.edges.len()
            && self.vis_f.len() == mesh.faces.len();
        if !fits || (!frozen && (changed || self.vis_key.as_ref() != Some(&key))) {
            self.visibility(camera, xray);
            self.vis_key = Some(key);
        }
    }

    fn visibility(&mut self, camera: &ViewCamera, xray: bool) {
        let hidden_edge = |[a, b]: [u32; 2]| self.hidden[a as usize] || self.hidden[b as usize];
        if xray {
            self.vis_v = self.hidden.iter().map(|h| !h).collect();
            self.vis_e = self.edges.iter().map(|e| !hidden_edge(*e)).collect();
            self.vis_f = self.face_hidden.iter().map(|h| !h).collect();
            return;
        }
        let shown: Vec<[u32; 3]> = self
            .tris
            .iter()
            .filter(|(f, _)| !self.face_hidden[*f])
            .map(|(_, t)| *t)
            .collect();
        let world: Vec<[Vec3; 3]> = shown
            .iter()
            .map(|t| t.map(|i| self.points[i as usize]))
            .collect();
        let screen: Vec<Option<[Pos2; 3]>> = shown
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.screen[i as usize]);
                Some([a?, b?, c?])
            })
            .collect();
        let occ = Occluders::new(world, &screen, camera.rect);
        let seen = |p: Vec3, at: Option<Pos2>| {
            let origin = if camera.orthographic {
                p - camera.forward * ((p - camera.eye).dot(camera.forward).max(0.0) + 1.0)
            } else {
                camera.eye
            };
            at.is_some_and(|at| !occ.occluded(origin, p, at))
        };
        self.vis_v = (0..self.points.len())
            .map(|i| !self.hidden[i] && seen(self.points[i], self.screen[i]))
            .collect();
        self.vis_e = self
            .edges
            .iter()
            .map(|&[a, b]| {
                let mid = (self.points[a as usize] + self.points[b as usize]) * 0.5;
                !hidden_edge([a, b]) && seen(mid, camera.project(mid))
            })
            .collect();
        self.vis_f = (0..self.centers.len())
            .map(|f| !self.face_hidden[f] && seen(self.centers[f], self.center_screen[f]))
            .collect();
    }

    /// Median of the selected vertices (world space).
    pub fn pivot(&self) -> Option<Vec3> {
        let (sum, n) = (0..self.points.len())
            .filter(|&i| self.selected[i])
            .fold((Vec3::ZERO, 0), |(s, n), i| (s + self.points[i], n + 1));
        (n > 0).then(|| sum / n as f32)
    }

    fn edge_screen(&self, [a, b]: [u32; 2]) -> Option<(Pos2, Pos2)> {
        Some((self.screen[a as usize]?, self.screen[b as usize]?))
    }

    pub fn pick(&self, mode: Mode, camera: &ViewCamera, p: Pos2, xray: bool) -> Option<Hover> {
        match mode {
            Mode::Vertex => (0..self.points.len())
                .filter(|&i| self.vis_v[i])
                .filter_map(|i| Some((i, self.screen[i]?.distance(p))))
                .filter(|(_, d)| *d < 10.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| Hover::Vertex(i as u32)),
            Mode::Edge => (0..self.edges.len())
                .filter(|&i| self.vis_e[i])
                .filter_map(|i| {
                    let (sa, sb) = self.edge_screen(self.edges[i])?;
                    let ab = sb - sa;
                    let t = ((p - sa).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.02, 0.98);
                    Some((i, t, p.distance(sa + ab * t)))
                })
                .filter(|e| e.2 < 8.0)
                .min_by(|x, y| x.2.total_cmp(&y.2))
                .map(|(i, t, _)| Hover::Edge(self.edges[i][0], self.edges[i][1], t)),
            Mode::Face => {
                let dot = (0..self.centers.len())
                    .filter(|&f| xray && self.vis_f[f])
                    .filter_map(|f| Some((f, self.center_screen[f]?.distance(p))))
                    .filter(|(_, d)| *d < 8.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((f, _)) = dot {
                    return Some(Hover::Face(f));
                }
                let ray = camera.ray(p);
                self.tris
                    .iter()
                    .filter(|(f, _)| !self.face_hidden[*f])
                    .filter_map(|(f, t)| {
                        let [a, b, c] = t.map(|i| self.points[i as usize]);
                        Some((*f, ray.triangle(a, b, c)?))
                    })
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(f, _)| Hover::Face(f))
            }
        }
    }

    /// Vertices of the elements inside a screen rectangle (box select).
    pub fn boxed(&self, mode: Mode, mesh: &PolyMesh, r: Rect) -> Vec<u32> {
        let mut out: Vec<u32> = match mode {
            Mode::Vertex => (0..self.points.len() as u32)
                .filter(|&i| {
                    self.vis_v[i as usize] && self.screen[i as usize].is_some_and(|s| r.contains(s))
                })
                .collect(),
            Mode::Edge => {
                let ends: Vec<Option<(Pos2, Pos2)>> = (0..self.edges.len())
                    .map(|i| self.edge_screen(self.edges[i]).filter(|_| self.vis_e[i]))
                    .collect();
                box_edges(&ends, r)
                    .into_iter()
                    .flat_map(|i| self.edges[i])
                    .collect()
            }
            Mode::Face => (0..self.centers.len())
                .filter(|&f| self.vis_f[f] && self.center_screen[f].is_some_and(|s| r.contains(s)))
                .flat_map(|f| mesh.faces[f].iter().copied())
                .collect(),
        };
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn draw(
        &self,
        painter: &egui::Painter,
        camera: &ViewCamera,
        mode: Mode,
        hover: Option<Hover>,
    ) {
        let tri = |t: [u32; 3]| -> Option<Vec<Pos2>> {
            let [a, b, c] = t.map(|i| self.screen[i as usize]);
            Some(vec![a?, b?, c?])
        };
        for &(f, t) in &self.tris {
            let hot = hover == Some(Hover::Face(f));
            let selected = mode == Mode::Face && self.face_selected[f] && !self.face_hidden[f];
            if !(hot || selected) {
                continue;
            }
            let fill = if hot {
                HOT.gamma_multiply(0.22)
            } else {
                theme::SELECTED.gamma_multiply(0.28)
            };
            if let Some(pts) = tri(t) {
                painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::NONE));
            }
        }
        for (i, &[a, b]) in self.edges.iter().enumerate() {
            let (ia, ib) = (a as usize, b as usize);
            if self.hidden[ia] || self.hidden[ib] {
                continue;
            }
            let on = self.selected[ia] && self.selected[ib];
            let (pa, pb) = (self.points[ia], self.points[ib]);
            if self.vis_e[i] {
                camera.line(painter, pa, pb, (2.5, WIRE));
                let stroke = if on {
                    (2.0, theme::SELECTED)
                } else {
                    (1.0, Color32::from_gray(200))
                };
                camera.line(painter, pa, pb, stroke);
            } else {
                let c = if on {
                    theme::SELECTED.gamma_multiply(0.4)
                } else {
                    Color32::from_white_alpha(40)
                };
                camera.line(painter, pa, pb, (1.0, c));
            }
        }
        if let Some(Hover::Edge(a, b, t)) = hover {
            let (pa, pb) = (self.points[a as usize], self.points[b as usize]);
            camera.line(painter, pa, pb, (3.0, HOT));
            if let Some(s) = camera.project(pa.lerp(pb, t)) {
                painter.circle_filled(s, 3.5, HOT);
            }
        }
        match mode {
            Mode::Vertex => {
                for i in (0..self.points.len()).filter(|&i| self.vis_v[i]) {
                    if let Some(s) = self.screen[i] {
                        let fill = if self.selected[i] {
                            theme::SELECTED
                        } else {
                            Color32::from_gray(15)
                        };
                        square(painter, s, 6.0, fill);
                    }
                }
            }
            Mode::Face => {
                for f in (0..self.centers.len()).filter(|&f| self.vis_f[f]) {
                    if let Some(s) = self.center_screen[f] {
                        let fill = if self.face_selected[f] {
                            theme::SELECTED
                        } else {
                            Color32::from_gray(15)
                        };
                        square(painter, s, 4.0, fill);
                    }
                }
            }
            Mode::Edge => {}
        }
        if let Some(Hover::Vertex(v)) = hover
            && let Some(s) = self.screen[v as usize]
        {
            painter.circle_stroke(s, 7.0, (2.0, HOT));
        }
    }
}

fn square(painter: &egui::Painter, at: Pos2, size: f32, fill: Color32) {
    let r = Rect::from_center_size(at, vec2(size, size));
    painter.rect_filled(r, 1.0, fill);
    painter.rect_stroke(
        r,
        1.0,
        (1.0, Color32::from_gray(220)),
        egui::StrokeKind::Middle,
    );
}
