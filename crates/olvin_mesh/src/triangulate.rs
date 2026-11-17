use glam::{Vec2, Vec3};

use crate::PolyMesh;
use crate::query::newell;

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Ear-clipping triangulation of a simple 2D polygon (either winding).
/// Returns triangles as indices into `points`, wound like the polygon.
/// Degenerate and self-intersecting input still yields `n - 2` triangles
/// at most, never panics.
pub fn ear_clip(points: &[Vec2]) -> Vec<[u32; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let signed_area: f32 = (0..n).map(|i| cross2(points[i], points[(i + 1) % n])).sum();
    // Work in CCW: flip the convexity test for clockwise input.
    let sign = if signed_area < 0.0 { -1.0 } else { 1.0 };
    let (min, max) = points.iter().fold((points[0], points[0]), |(lo, hi), &p| {
        (lo.min(p), hi.max(p))
    });
    let eps = (max - min).length_squared() * 1e-9;

    let mut idx: Vec<usize> = (0..n).collect();
    let mut tris = Vec::with_capacity(n - 2);
    while idx.len() > 3 {
        let m = idx.len();
        let corner = |i: usize| {
            let (a, b, c) = (
                points[idx[(i + m - 1) % m]],
                points[idx[i]],
                points[idx[(i + 1) % m]],
            );
            (a, b, c, sign * cross2(b - a, c - b))
        };
        let is_ear = |i: usize| {
            let (a, b, c, turn) = corner(i);
            if turn <= eps {
                return false;
            }
            // No other remaining vertex strictly inside the candidate triangle.
            (0..m)
                .filter(|&j| j != i && j != (i + m - 1) % m && j != (i + 1) % m)
                .all(|j| {
                    let p = points[idx[j]];
                    let inside = sign * cross2(b - a, p - a) > eps
                        && sign * cross2(c - b, p - b) > eps
                        && sign * cross2(a - c, p - c) > eps;
                    !inside
                })
        };
        let pick = match (0..m).find(|&i| is_ear(i)) {
            Some(i) => Some(i),
            None => {
                // Collinear vertices add no area: drop one without a triangle.
                if let Some(i) = (0..m).find(|&i| corner(i).3.abs() <= eps) {
                    idx.remove(i);
                    continue;
                }
                // Self-intersecting input: clip the most convex corner.
                (0..m).max_by(|&i, &j| corner(i).3.total_cmp(&corner(j).3))
            }
        };
        let i = pick.unwrap_or(0);
        tris.push([
            idx[(i + m - 1) % m] as u32,
            idx[i] as u32,
            idx[(i + 1) % m] as u32,
        ]);
        idx.remove(i);
    }
    tris.push([idx[0] as u32, idx[1] as u32, idx[2] as u32]);
    tris
}

impl PolyMesh {
    /// Triangulated face as indices into `self.positions` (shared vertices).
    pub fn face_triangles(&self, f: usize) -> Vec<[u32; 3]> {
        let face = &self.faces[f];
        self.face_triangles_local(f)
            .into_iter()
            .map(|t| t.map(|i| face[i as usize]))
            .collect()
    }

    /// Triangles as positions within the face loop.
    fn face_triangles_local(&self, f: usize) -> Vec<[u32; 3]> {
        if self.faces[f].len() == 3 {
            return vec![[0, 1, 2]];
        }
        let normal = newell(self.face_positions(f)).normalize_or_zero();
        let normal = if normal == Vec3::ZERO {
            Vec3::Y
        } else {
            normal
        };
        let u = normal.any_orthonormal_vector();
        let v = normal.cross(u);
        let pts: Vec<Vec2> = self
            .face_positions(f)
            .map(|p| Vec2::new(p.dot(u), p.dot(v)))
            .collect();
        ear_clip(&pts)
    }

    /// Flat-shaded render data: every face gets its own copies of its
    /// vertices with the face normal. Concave faces are ear-clipped in their
    /// plane. Degenerate faces get a +Y normal.
    pub fn triangulate(&self) -> (Vec<Vec3>, Vec<Vec3>, Vec<u32>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut indices = Vec::new();
        for (f, face) in self.faces.iter().enumerate() {
            let base = positions.len() as u32;
            let n = self.face_normal(f);
            let n = if n == Vec3::ZERO { Vec3::Y } else { n };
            positions.extend(self.face_positions(f));
            normals.extend(std::iter::repeat_n(n, face.len()));
            for t in self.face_triangles_local(f) {
                indices.extend(t.map(|i| base + i));
            }
        }
        (positions, normals, indices)
    }
}
