//! Selection in the UV view (Blender with UV sync off): face corners are
//! selected; a UV vertex is every corner of one mesh vertex at the same UV
//! ("shared location"), a face is selected when all its corners are.
//! Everything here is pure: hit tests take a UV -> screen mapping.

use std::collections::{BTreeSet, HashSet};

use olvin_editor::glam::Vec2;
use olvin_mesh::{PolyMesh, UvCorner, ear_clip, edge_key};

/// Corners of one vertex closer than this in UV are one UV vertex.
pub const STICKY_EPS: f32 = 1e-5;
/// Pick radii in screen points.
pub const VERTEX_RADIUS: f32 = 10.0;
pub const EDGE_RADIUS: f32 = 7.0;
pub const FACE_DOT_RADIUS: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectMode {
    #[default]
    Vertex,
    Edge,
    Face,
    Island,
}

impl SelectMode {
    pub const ALL: [(SelectMode, &'static str); 4] = [
        (SelectMode::Vertex, "Vertex 1"),
        (SelectMode::Edge, "Edge 2"),
        (SelectMode::Face, "Face 3"),
        (SelectMode::Island, "Island 4"),
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    Replace,
    Add,
    Subtract,
    /// Shift+click: remove if everything picked is selected, else add.
    Toggle,
}

/// What the cursor is over. `Edge((f, i))` is the UV edge from corner `i`
/// of face `f` to corner `i + 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Vertex(UvCorner),
    Edge(UvCorner),
    Face(usize),
}

/// Faces shown in the view and corner lookup tables. The mesh must have UVs.
pub struct Topo<'a> {
    pub mesh: &'a PolyMesh,
    pub visible: Vec<usize>,
    shown: Vec<bool>,
    by_vertex: Vec<Vec<UvCorner>>,
}

impl<'a> Topo<'a> {
    pub fn new(mesh: &'a PolyMesh, visible: Vec<usize>) -> Self {
        let mut shown = vec![false; mesh.faces.len()];
        let visible: Vec<usize> = visible
            .into_iter()
            .filter(|&f| {
                f < mesh.faces.len()
                    && f < mesh.uvs.len()
                    && !std::mem::replace(&mut shown[f], true)
            })
            .collect();
        let mut by_vertex = vec![Vec::new(); mesh.positions.len()];
        for &f in &visible {
            for (i, &v) in mesh.faces[f].iter().enumerate() {
                by_vertex[v as usize].push((f, i));
            }
        }
        Self {
            mesh,
            visible,
            shown,
            by_vertex,
        }
    }

    pub fn is_shown(&self, f: usize) -> bool {
        self.shown.get(f).copied().unwrap_or(false)
    }

    pub fn uv(&self, (f, i): UvCorner) -> Vec2 {
        self.mesh.uvs[f][i]
    }

    /// The corner after `c` in its face.
    pub fn next(&self, (f, i): UvCorner) -> UvCorner {
        (f, (i + 1) % self.mesh.faces[f].len())
    }

    /// Every shown corner of the same vertex at the same UV (the UV vertex).
    pub fn cluster(&self, c: UvCorner) -> Vec<UvCorner> {
        let v = self.mesh.faces[c.0][c.1] as usize;
        let at = self.uv(c);
        self.by_vertex[v]
            .iter()
            .copied()
            .filter(|&d| self.uv(d).distance(at) <= STICKY_EPS)
            .collect()
    }

    /// Corners grown to whole UV vertices: what a transform moves so
    /// connected UVs stay connected.
    pub fn sticky(&self, corners: impl IntoIterator<Item = UvCorner>) -> Vec<UvCorner> {
        let mut out = BTreeSet::new();
        for c in corners {
            if self.is_shown(c.0) {
                out.extend(self.cluster(c));
            }
        }
        out.into_iter().collect()
    }

    pub fn corners(&self) -> Vec<UvCorner> {
        self.mesh.face_corners(&self.visible)
    }

    pub fn face_corners(&self, faces: &[usize]) -> Vec<UvCorner> {
        let shown: Vec<usize> = faces
            .iter()
            .copied()
            .filter(|&f| self.is_shown(f))
            .collect();
        self.mesh.face_corners(&shown)
    }

    pub fn center(&self, f: usize) -> Vec2 {
        let uv = &self.mesh.uvs[f];
        uv.iter().sum::<Vec2>() / uv.len().max(1) as f32
    }

    /// Shown faces of the UV islands containing `faces`.
    pub fn islands_of(&self, faces: &[usize]) -> Vec<usize> {
        let want: HashSet<usize> = faces.iter().copied().collect();
        let mut out: Vec<usize> = self
            .mesh
            .islands()
            .into_iter()
            .filter(|isl| isl.iter().any(|f| want.contains(f)))
            .flatten()
            .filter(|&f| self.is_shown(f))
            .collect();
        out.sort_unstable();
        out
    }

    /// Corners selected by clicking `hit` in `mode`.
    pub fn expand(&self, hit: Hit, mode: SelectMode) -> Vec<UvCorner> {
        match (hit, mode) {
            (Hit::Vertex(c), _) => self.cluster(c),
            (Hit::Edge(c), _) => {
                let mut out = self.cluster(c);
                out.extend(self.cluster(self.next(c)));
                out
            }
            (Hit::Face(f), SelectMode::Island) => self.face_corners(&self.islands_of(&[f])),
            (Hit::Face(f), _) => self.face_corners(&[f]),
        }
    }

    /// Corners picked by a box (`inside` tests UV points): vertices inside,
    /// edges with both ends inside, faces (islands) whose centre is inside.
    pub fn boxed(&self, mode: SelectMode, inside: impl Fn(Vec2) -> bool) -> Vec<UvCorner> {
        match mode {
            SelectMode::Vertex => self
                .corners()
                .into_iter()
                .filter(|&c| inside(self.uv(c)))
                .collect(),
            SelectMode::Edge => {
                let mut out = BTreeSet::new();
                for c in self.corners() {
                    let n = self.next(c);
                    if inside(self.uv(c)) && inside(self.uv(n)) {
                        out.extend([c, n]);
                    }
                }
                out.into_iter().collect()
            }
            SelectMode::Face | SelectMode::Island => {
                let faces: Vec<usize> = self
                    .visible
                    .iter()
                    .copied()
                    .filter(|&f| inside(self.center(f)))
                    .collect();
                let faces = if mode == SelectMode::Island {
                    self.islands_of(&faces)
                } else {
                    faces
                };
                self.face_corners(&faces)
            }
        }
    }

    /// The element under `mouse` (screen points); `to_screen` maps UVs.
    pub fn pick(
        &self,
        mode: SelectMode,
        to_screen: impl Fn(Vec2) -> Vec2,
        mouse: Vec2,
    ) -> Option<Hit> {
        match mode {
            SelectMode::Vertex => self
                .corners()
                .into_iter()
                .map(|c| (c, to_screen(self.uv(c)).distance(mouse)))
                .filter(|(_, d)| *d <= VERTEX_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(c, _)| Hit::Vertex(c)),
            SelectMode::Edge => self
                .corners()
                .into_iter()
                .map(|c| {
                    let (a, b) = (to_screen(self.uv(c)), to_screen(self.uv(self.next(c))));
                    (c, segment_distance(mouse, a, b))
                })
                .filter(|(_, d)| *d <= EDGE_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(c, _)| Hit::Edge(c)),
            SelectMode::Face | SelectMode::Island => {
                let screen = |f: usize| -> Vec<Vec2> {
                    self.mesh.uvs[f].iter().map(|&t| to_screen(t)).collect()
                };
                // The smallest face containing the cursor, else the nearest centre dot.
                let inside = self
                    .visible
                    .iter()
                    .copied()
                    .filter_map(|f| {
                        let poly = screen(f);
                        point_in_polygon(mouse, &poly).then(|| (f, polygon_area(&poly).abs()))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(f, _)| f);
                inside
                    .or_else(|| {
                        self.visible
                            .iter()
                            .map(|&f| (f, to_screen(self.center(f)).distance(mouse)))
                            .filter(|(_, d)| *d <= FACE_DOT_RADIUS)
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(f, _)| f)
                    })
                    .map(Hit::Face)
            }
        }
    }

    /// Triangles of a face in UV space (concave faces ear-clipped).
    pub fn triangles(&self, f: usize) -> Vec<[u32; 3]> {
        ear_clip(&self.mesh.uvs[f])
    }
}

/// Selected face corners.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub corners: BTreeSet<UvCorner>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.corners.is_empty()
    }

    pub fn contains(&self, c: UvCorner) -> bool {
        self.corners.contains(&c)
    }

    pub fn clear(&mut self) {
        self.corners.clear();
    }

    /// Drops corners that no longer exist (after undo or topology edits).
    pub fn sanitize(&mut self, mesh: &PolyMesh) {
        self.corners
            .retain(|&(f, i)| mesh.faces.get(f).is_some_and(|face| i < face.len()));
    }

    pub fn combine(&mut self, picked: &[UvCorner], how: Combine) {
        match how {
            Combine::Replace => {
                self.corners.clear();
                self.corners.extend(picked.iter().copied());
            }
            Combine::Add => self.corners.extend(picked.iter().copied()),
            Combine::Subtract => {
                for c in picked {
                    self.corners.remove(c);
                }
            }
            Combine::Toggle => {
                let all = !picked.is_empty() && picked.iter().all(|c| self.corners.contains(c));
                self.combine(picked, if all { Combine::Subtract } else { Combine::Add });
            }
        }
    }

    /// Shown faces whose corners are all selected.
    pub fn faces(&self, topo: &Topo) -> Vec<usize> {
        topo.visible
            .iter()
            .copied()
            .filter(|&f| (0..topo.mesh.faces[f].len()).all(|i| self.contains((f, i))))
            .collect()
    }

    /// Mesh edges whose UV edge has both corners selected, sorted.
    pub fn edges(&self, topo: &Topo) -> Vec<[u32; 2]> {
        let mut out: Vec<[u32; 2]> = self
            .corners
            .iter()
            .copied()
            .filter(|c| topo.is_shown(c.0))
            .filter(|&c| self.contains(topo.next(c)))
            .map(|(f, i)| {
                let face = &topo.mesh.faces[f];
                edge_key(face[i], face[(i + 1) % face.len()])
            })
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether the UV edge starting at corner `c` is selected.
    pub fn has_edge(&self, topo: &Topo, c: UvCorner) -> bool {
        self.contains(c) && self.contains(topo.next(c))
    }
}

pub fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

pub fn point_in_polygon(p: Vec2, poly: &[Vec2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            inside = !inside;
        }
    }
    inside
}

pub fn polygon_area(poly: &[Vec2]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| poly[i].perp_dot(poly[(i + 1) % n]))
        .sum::<f32>()
        * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x1 grid of quads with one shared edge, UVs = XZ position.
    fn strip() -> PolyMesh {
        let mut m = PolyMesh::plane(2.0);
        let faces = m.subdivide_faces(&[0], 1);
        assert_eq!(faces.len(), 4);
        m.project_planar(&faces, None, 0.5);
        m
    }

    fn ident(t: Vec2) -> Vec2 {
        t * 100.0
    }

    #[test]
    fn clusters_join_shared_corners_only_at_same_uv() {
        let mut m = strip();
        let topo = Topo::new(&m, (0..4).collect());
        // The centre vertex is shared by all four faces at one UV.
        let centre = m.positions.iter().position(|p| p.length() < 1e-6).unwrap() as u32;
        let c = topo.by_vertex[centre as usize][0];
        assert_eq!(topo.cluster(c).len(), 4);
        // Split face 0 off in UV: its corner leaves the cluster.
        let shift: Vec<UvCorner> = m.face_corners(&[0]);
        m.transform_uvs(
            &shift,
            olvin_editor::glam::Affine2::from_translation(Vec2::X),
        );
        let topo = Topo::new(&m, (0..4).collect());
        let c = (1..4)
            .find_map(|f| m.faces[f].iter().position(|&v| v == centre).map(|i| (f, i)))
            .unwrap();
        assert_eq!(topo.cluster(c).len(), 3);
        assert_eq!(topo.islands_of(&[0]), vec![0]);
        assert_eq!(topo.islands_of(&[1]), vec![1, 2, 3]);
    }

    #[test]
    fn pick_vertex_edge_face() {
        let m = strip();
        let topo = Topo::new(&m, (0..4).collect());
        // UV (0.5, 0.5) is the centre vertex (screen 50, 50).
        let hit = topo.pick(SelectMode::Vertex, ident, Vec2::new(52.0, 49.0));
        let Some(Hit::Vertex(c)) = hit else {
            panic!("{hit:?}")
        };
        assert!(topo.uv(c).abs_diff_eq(Vec2::splat(0.5), 1e-5));
        assert_eq!(
            topo.pick(SelectMode::Vertex, ident, Vec2::new(25.0, 25.0)),
            None
        );
        let hit = topo.pick(SelectMode::Edge, ident, Vec2::new(50.0, 20.0));
        let Some(Hit::Edge(e)) = hit else {
            panic!("{hit:?}")
        };
        let (a, b) = (topo.uv(e), topo.uv(topo.next(e)));
        assert!(
            (a.x - 0.5).abs() < 1e-5 && (b.x - 0.5).abs() < 1e-5,
            "vertical edge at u=0.5"
        );
        let Some(Hit::Face(f)) = topo.pick(SelectMode::Face, ident, Vec2::new(75.0, 80.0)) else {
            panic!()
        };
        assert!(topo.center(f).abs_diff_eq(Vec2::new(0.75, 0.75), 1e-5));
        assert_eq!(
            topo.pick(SelectMode::Face, ident, Vec2::new(500.0, 80.0)),
            None
        );
    }

    #[test]
    fn expand_box_and_derived_elements() {
        let m = strip();
        let topo = Topo::new(&m, (0..4).collect());
        let mut sel = Selection::default();
        let Some(hit) = topo.pick(SelectMode::Face, ident, Vec2::new(25.0, 25.0)) else {
            panic!()
        };
        sel.combine(&topo.expand(hit, SelectMode::Face), Combine::Replace);
        assert_eq!(sel.faces(&topo).len(), 1);
        assert_eq!(sel.edges(&topo).len(), 4);
        // Shift+click on the same face toggles it off.
        sel.combine(&topo.expand(hit, SelectMode::Face), Combine::Toggle);
        assert!(sel.is_empty());
        // Island mode picks the whole connected layout.
        sel.combine(&topo.expand(hit, SelectMode::Island), Combine::Replace);
        assert_eq!(sel.faces(&topo).len(), 4);
        assert_eq!(sel.edges(&topo).len(), 12);
        // Box over the left half: vertices with u <= 0.5.
        let left = |t: Vec2| t.x <= 0.5 + 1e-5;
        let picked = topo.boxed(SelectMode::Vertex, left);
        assert!(picked.iter().all(|&c| left(topo.uv(c))));
        assert_eq!(picked.len(), 12, "corners of x<=0 vertices over four faces");
        sel.combine(&picked, Combine::Replace);
        assert_eq!(sel.faces(&topo).len(), 2);
        let faces = topo.boxed(SelectMode::Face, left);
        assert_eq!(faces.len(), 8);
        sel.combine(&faces, Combine::Subtract);
        // Left: the right faces' corners on the u = 0.5 column.
        assert_eq!(sel.corners.len(), 4);
        assert!(
            sel.corners
                .iter()
                .all(|&c| (topo.uv(c).x - 0.5).abs() < 1e-5)
        );
        // Sticky growth keeps the centre UV vertex whole.
        let one = vec![(0usize, 0usize)];
        let grown = topo.sticky(one);
        assert!(grown.contains(&(0, 0)));
        assert!(grown.iter().all(|&c| topo.uv(c) == topo.uv((0, 0))));
    }

    #[test]
    fn hidden_faces_are_ignored_and_sanitize() {
        let m = strip();
        let topo = Topo::new(&m, vec![1, 1, 99]);
        assert_eq!(topo.visible, vec![1]);
        assert_eq!(topo.corners().len(), 4);
        let mut sel = Selection::default();
        sel.combine(&[(0, 0), (1, 0), (7, 0), (1, 9)], Combine::Add);
        sel.sanitize(&m);
        assert_eq!(sel.corners.len(), 2);
        assert!(
            topo.sticky(sel.corners.iter().copied())
                .iter()
                .all(|c| c.0 == 1)
        );
    }

    #[test]
    fn geometry_helpers() {
        let sq = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
        assert!(point_in_polygon(Vec2::splat(0.5), &sq));
        assert!(!point_in_polygon(Vec2::new(1.5, 0.5), &sq));
        assert_eq!(polygon_area(&sq), 1.0);
        assert_eq!(
            segment_distance(Vec2::new(0.5, 1.0), Vec2::ZERO, Vec2::X),
            1.0
        );
        assert_eq!(
            segment_distance(Vec2::new(-3.0, 4.0), Vec2::ZERO, Vec2::X),
            5.0
        );
    }
}
