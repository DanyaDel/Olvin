//! Constructive solid geometry over ordered convex brushes, the way level
//! editors (Hammer, Realtime CSG) do it.
//!
//! Every face of every brush is split by the planes of the brushes it
//! touches, so each fragment is entirely inside or outside each of them.
//! A fragment is kept when the solid on its two sides differs after applying
//! all operations in order; which side is solid gives its orientation. This
//! point-sampling is robust to touching and coplanar faces, which plague
//! BSP-based CSG.

mod shapes;

use glam::{Mat4, Vec3};
pub use shapes::{BrushShape, Face, Piece, pieces};

/// Distance below which a point is considered on a plane.
const PLANE_EPS: f32 = 1e-4;
/// Offset of the solidity samples from a fragment.
const SAMPLE_EPS: f32 = 2e-3;
const MIN_AREA: f32 = 1e-7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CsgOp {
    #[default]
    Add,
    Subtract,
    Intersect,
}

impl CsgOp {
    pub const ALL: [CsgOp; 3] = [CsgOp::Add, CsgOp::Subtract, CsgOp::Intersect];

    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "Additive",
            Self::Subtract => "Subtractive",
            Self::Intersect => "Intersecting",
        }
    }
}

/// One brush, already placed in the world. Order in the input slice is the
/// order operations apply in.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushInput {
    pub id: u64,
    pub shape: BrushShape,
    pub op: CsgOp,
    pub transform: Mat4,
    pub color: [f32; 4],
}

/// A surviving convex surface polygon, counter-clockwise when seen from the
/// empty side (its normal points out of the solid).
#[derive(Clone, Debug)]
pub struct Polygon {
    pub vertices: Vec<Vec3>,
    pub normal: Vec3,
    pub brush: u64,
    pub color: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    n: Vec3,
    d: f32,
}

impl Plane {
    fn from_points(points: &[Vec3]) -> Self {
        // Newell's method: robust for any planar loop.
        let mut n = Vec3::ZERO;
        for (i, a) in points.iter().enumerate() {
            let b = points[(i + 1) % points.len()];
            n += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        let n = n.normalize_or_zero();
        let centroid = points.iter().copied().sum::<Vec3>() / points.len() as f32;
        Self { n, d: n.dot(centroid) }
    }

    fn dist(&self, p: Vec3) -> f32 {
        self.n.dot(p) - self.d
    }
}

struct WorldFace {
    vertices: Vec<Vec3>,
    plane: Plane,
}

struct WorldPiece {
    brush: usize,
    faces: Vec<WorldFace>,
    min: Vec3,
    max: Vec3,
}

impl WorldPiece {
    fn new(brush: usize, local: &Piece, transform: Mat4) -> Option<Self> {
        let faces_world: Vec<Vec<Vec3>> =
            local.iter().map(|f| f.iter().map(|v| transform.transform_point3(*v)).collect()).collect();
        let all: Vec<Vec3> = faces_world.iter().flatten().copied().collect();
        let center = all.iter().copied().sum::<Vec3>() / all.len() as f32;
        let mut faces = Vec::with_capacity(faces_world.len());
        for mut vertices in faces_world {
            let mut plane = Plane::from_points(&vertices);
            if plane.n == Vec3::ZERO {
                continue; // degenerate (zero scale on some axis)
            }
            // Mirroring transforms flip winding: orient every face outwards.
            if plane.dist(center) > 0.0 {
                vertices.reverse();
                plane = Plane::from_points(&vertices);
            }
            faces.push(WorldFace { vertices, plane });
        }
        if faces.len() < 4 {
            return None;
        }
        let min = all.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min);
        let max = all.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
        Some(Self { brush, faces, min, max })
    }

    fn contains(&self, p: Vec3) -> bool {
        self.faces.iter().all(|f| f.plane.dist(p) < 0.0)
    }

    fn overlaps(&self, other: &WorldPiece) -> bool {
        let e = Vec3::splat(PLANE_EPS * 10.0);
        (self.min - e).cmple(other.max).all() && (other.min - e).cmple(self.max).all()
    }

    /// On the face's plane and within the other faces' bounds.
    fn face_covers(&self, face: usize, p: Vec3) -> bool {
        self.faces.iter().enumerate().all(|(i, f)| i == face || f.plane.dist(p) <= PLANE_EPS * 10.0)
    }
}

/// Splits a convex polygon by a plane into (front, back) parts. Coplanar
/// polygons go to the back.
fn split(poly: &[Vec3], plane: &Plane) -> (Vec<Vec3>, Vec<Vec3>) {
    let dists: Vec<f32> = poly.iter().map(|p| plane.dist(*p)).collect();
    if dists.iter().all(|d| *d <= PLANE_EPS) {
        return (Vec::new(), poly.to_vec());
    }
    if dists.iter().all(|d| *d >= -PLANE_EPS) {
        return (poly.to_vec(), Vec::new());
    }
    let (mut front, mut back) = (Vec::new(), Vec::new());
    for i in 0..poly.len() {
        let j = (i + 1) % poly.len();
        let (a, b, da, db) = (poly[i], poly[j], dists[i], dists[j]);
        if da >= -PLANE_EPS {
            front.push(a);
        }
        if da <= PLANE_EPS {
            back.push(a);
        }
        if (da > PLANE_EPS && db < -PLANE_EPS) || (da < -PLANE_EPS && db > PLANE_EPS) {
            let p = a + (b - a) * (da / (da - db));
            front.push(p);
            back.push(p);
        }
    }
    (front, back)
}

fn area(poly: &[Vec3]) -> f32 {
    let mut cross = Vec3::ZERO;
    for i in 1..poly.len().saturating_sub(1) {
        cross += (poly[i] - poly[0]).cross(poly[i + 1] - poly[0]);
    }
    cross.length() * 0.5
}

/// Evaluates the brushes into surface polygons.
pub fn build(brushes: &[BrushInput]) -> Vec<Polygon> {
    let pieces: Vec<WorldPiece> = brushes
        .iter()
        .enumerate()
        .flat_map(|(i, b)| pieces(b.shape).into_iter().filter_map(move |p| WorldPiece::new(i, &p, b.transform)))
        .collect();

    let solid = |p: Vec3| {
        let mut s = false;
        let mut i = 0;
        while i < pieces.len() {
            // Pieces of one brush are contiguous; a brush contains p if any piece does.
            let brush = pieces[i].brush;
            let mut inside = false;
            while i < pieces.len() && pieces[i].brush == brush {
                inside |= pieces[i].contains(p);
                i += 1;
            }
            s = match brushes[brush].op {
                CsgOp::Add => s || inside,
                CsgOp::Subtract => s && !inside,
                CsgOp::Intersect => s && inside,
            };
        }
        s
    };

    let mut out = Vec::new();
    for (pi, piece) in pieces.iter().enumerate() {
        let brush = &brushes[piece.brush];
        let neighbours: Vec<usize> =
            (0..pieces.len()).filter(|&qi| qi != pi && piece.overlaps(&pieces[qi])).collect();

        for face in &piece.faces {
            let mut fragments = vec![face.vertices.clone()];
            for &qi in &neighbours {
                let mut next = Vec::with_capacity(fragments.len());
                for fragment in fragments {
                    let mut rest = fragment;
                    for plane in pieces[qi].faces.iter().map(|f| &f.plane) {
                        let (front, back) = split(&rest, plane);
                        if front.len() >= 3 {
                            next.push(front);
                        }
                        rest = back;
                        if rest.len() < 3 {
                            break;
                        }
                    }
                    if rest.len() >= 3 {
                        next.push(rest);
                    }
                }
                fragments = next;
            }

            let n = face.plane.n;
            for fragment in fragments {
                if area(&fragment) < MIN_AREA {
                    continue;
                }
                let c = fragment.iter().copied().sum::<Vec3>() / fragment.len() as f32;
                let (front, back) = (solid(c + n * SAMPLE_EPS), solid(c - n * SAMPLE_EPS));
                if front == back {
                    continue;
                }
                // Coplanar, same-facing faces of later pieces produce the same
                // fragment: keep only the last one.
                let duplicated = pieces[pi + 1..].iter().any(|q| {
                    q.faces.iter().enumerate().any(|(fi, f)| {
                        f.plane.n.dot(n) > 0.9999 && (f.plane.d - face.plane.d).abs() < PLANE_EPS * 10.0 && q.face_covers(fi, c)
                    })
                });
                if duplicated {
                    continue;
                }
                let (vertices, normal) = if back { (fragment, n) } else { (fragment.into_iter().rev().collect(), -n) };
                out.push(Polygon { vertices, normal, brush: brush.id, color: brush.color });
            }
        }
    }
    out
}

/// Outline edges of a brush in world space, for editor wireframes.
pub fn brush_edges(shape: BrushShape, transform: Mat4) -> Vec<[Vec3; 2]> {
    let mut edges: Vec<[Vec3; 2]> = Vec::new();
    for piece in pieces(shape) {
        for face in piece {
            for i in 0..face.len() {
                let (a, b) = (face[i], face[(i + 1) % face.len()]);
                let dup = edges.iter().any(|e| {
                    (e[0].abs_diff_eq(a, 1e-5) && e[1].abs_diff_eq(b, 1e-5)) || (e[0].abs_diff_eq(b, 1e-5) && e[1].abs_diff_eq(a, 1e-5))
                });
                if !dup {
                    edges.push([a, b]);
                }
            }
        }
    }
    edges.into_iter().map(|[a, b]| [transform.transform_point3(a), transform.transform_point3(b)]).collect()
}

/// Indexed triangle mesh of the polygons (fan triangulation, flat normals).
#[derive(Clone, Debug, Default)]
pub struct TriMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl TriMesh {
    pub fn from_polygons<'a>(polygons: impl IntoIterator<Item = &'a Polygon>) -> Self {
        let mut mesh = Self::default();
        for p in polygons {
            let base = mesh.positions.len() as u32;
            mesh.positions.extend(&p.vertices);
            mesh.normals.extend(std::iter::repeat_n(p.normal, p.vertices.len()));
            for i in 1..p.vertices.len() as u32 - 1 {
                mesh.indices.extend([base, base + i, base + i + 1]);
            }
        }
        mesh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brush(id: u64, op: CsgOp, center: Vec3, size: Vec3) -> BrushInput {
        BrushInput {
            id,
            shape: BrushShape::Box,
            op,
            transform: Mat4::from_scale_rotation_translation(size, glam::Quat::IDENTITY, center),
            color: [1.0; 4],
        }
    }

    fn total_area(polys: &[Polygon]) -> f32 {
        polys.iter().map(|p| area(&p.vertices)).sum()
    }

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }

    #[test]
    fn single_box_has_six_outward_faces() {
        let polys = build(&[brush(1, CsgOp::Add, Vec3::ZERO, Vec3::splat(2.0))]);
        assert_eq!(polys.len(), 6);
        assert_close(total_area(&polys), 24.0);
        for p in &polys {
            let c = p.vertices.iter().copied().sum::<Vec3>() / p.vertices.len() as f32;
            assert!(p.normal.dot(c) > 0.0, "normal must point outwards");
            assert!(Plane::from_points(&p.vertices).n.dot(p.normal) > 0.99, "winding matches normal");
        }
    }

    #[test]
    fn touching_boxes_hide_shared_face() {
        // Two 1x1x1 boxes side by side form a 2x1x1 box: area 10, no internal faces.
        let polys = build(&[
            brush(1, CsgOp::Add, Vec3::new(-0.5, 0.0, 0.0), Vec3::ONE),
            brush(2, CsgOp::Add, Vec3::new(0.5, 0.0, 0.0), Vec3::ONE),
        ]);
        assert_close(total_area(&polys), 10.0);
    }

    #[test]
    fn overlapping_coplanar_faces_are_not_duplicated() {
        // Same box twice: still exactly one surface.
        let b = brush(1, CsgOp::Add, Vec3::ZERO, Vec3::ONE);
        let polys = build(&[b.clone(), BrushInput { id: 2, ..b }]);
        assert_close(total_area(&polys), 6.0);
    }

    #[test]
    fn subtract_makes_a_room() {
        // 4x4x4 solid, 2x2x2 hole fully inside: outer 96 + inner 24 (facing in).
        let polys = build(&[
            brush(1, CsgOp::Add, Vec3::ZERO, Vec3::splat(4.0)),
            brush(2, CsgOp::Subtract, Vec3::ZERO, Vec3::splat(2.0)),
        ]);
        assert_close(total_area(&polys), 96.0 + 24.0);
        let inner: Vec<_> = polys.iter().filter(|p| p.brush == 2).collect();
        assert_eq!(inner.len(), 6);
        for p in inner {
            let c = p.vertices.iter().copied().sum::<Vec3>() / p.vertices.len() as f32;
            assert!(p.normal.dot(c) < 0.0, "room walls face the inside");
        }
    }

    #[test]
    fn subtract_through_cuts_a_tunnel() {
        // 2x2x2 box with a 1x1x4 hole along Z: 24 - 2*1 + 4 tunnel walls of 1x2.
        let polys = build(&[
            brush(1, CsgOp::Add, Vec3::ZERO, Vec3::splat(2.0)),
            brush(2, CsgOp::Subtract, Vec3::ZERO, Vec3::new(1.0, 1.0, 4.0)),
        ]);
        assert_close(total_area(&polys), 24.0 - 2.0 + 8.0);
    }

    #[test]
    fn order_matters_and_intersect_works() {
        // Subtract before add does nothing.
        let polys = build(&[
            brush(2, CsgOp::Subtract, Vec3::ZERO, Vec3::ONE),
            brush(1, CsgOp::Add, Vec3::ZERO, Vec3::splat(2.0)),
        ]);
        assert_close(total_area(&polys), 24.0);
        // Intersection of two offset 2-cubes is a 1x2x2 box.
        let polys = build(&[
            brush(1, CsgOp::Add, Vec3::ZERO, Vec3::splat(2.0)),
            brush(2, CsgOp::Intersect, Vec3::new(1.0, 0.0, 0.0), Vec3::splat(2.0)),
        ]);
        assert_close(total_area(&polys), 2.0 * (2.0 + 2.0 + 4.0));
    }

    #[test]
    fn shapes_are_closed_convex_pieces() {
        for shape in [
            BrushShape::Box,
            BrushShape::Wedge,
            BrushShape::Cylinder { sides: 12 },
            BrushShape::Pyramid { sides: 4 },
            BrushShape::Stairs { steps: 4 },
        ] {
            let polys = build(&[BrushInput { id: 1, shape, op: CsgOp::Add, transform: Mat4::IDENTITY, color: [1.0; 4] }]);
            assert!(!polys.is_empty(), "{shape:?}");
            // Closed surface: the oriented normals integrate to zero.
            let flux: Vec3 = polys.iter().map(|p| p.normal * area(&p.vertices)).sum();
            assert!(flux.length() < 1e-3, "{shape:?} not closed: {flux}");
        }
        // Wedge volume check via area: bottom 1 + back 1 + slope sqrt2 + 2 * 0.5.
        let wedge = build(&[BrushInput { id: 1, shape: BrushShape::Wedge, op: CsgOp::Add, transform: Mat4::IDENTITY, color: [1.0; 4] }]);
        assert_close(total_area(&wedge), 3.0 + 2f32.sqrt());
        // Stairs: internal faces between steps are removed.
        let stairs = build(&[BrushInput { id: 1, shape: BrushShape::Stairs { steps: 2 }, op: CsgOp::Add, transform: Mat4::IDENTITY, color: [1.0; 4] }]);
        // bottom 1, back 1, sides 2*(0.5*1 + 0.5*0.5)=1.5, treads 0.5+0.5, risers 0.5+0.5
        assert_close(total_area(&stairs), 1.0 + 1.0 + 1.5 + 1.0 + 1.0);
    }

    #[test]
    fn mirrored_brush_still_faces_outwards() {
        let mirrored = BrushInput {
            id: 1,
            shape: BrushShape::Wedge,
            op: CsgOp::Add,
            transform: Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
            color: [1.0; 4],
        };
        let polys = build(&[mirrored]);
        let flux: Vec3 = polys.iter().map(|p| p.normal * area(&p.vertices)).sum();
        assert!(flux.length() < 1e-3);
        for p in &polys {
            assert!(Plane::from_points(&p.vertices).n.dot(p.normal) > 0.99);
        }
    }
}
