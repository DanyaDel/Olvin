//! Per-face-corner UVs: helpers that keep them consistent through the
//! modelling operations, projections, seams, islands and transforms. The
//! real unwrap (LSCM) and island packing live in `unwrap.rs`.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{PI, TAU};

use glam::{Affine2, Mat4, Vec2, Vec3};

use crate::extrude::unique_faces;
use crate::query::find_directed;
use crate::{PolyMesh, edge_key, loop_edges};

/// A face corner: `(face, index in the face loop)`. `uvs[face][index]` is its UV.
pub type UvCorner = (usize, usize);

/// Two UVs closer than this are the same point (island connectivity).
pub(crate) const UV_EPS: f32 = 1e-5;

/// Tangent basis `(u, v)` of a plane with normal `n`, with `u × v = n`, so
/// projecting onto it never mirrors a face seen from the front. For an axis
/// normal it matches Blender's cube projection (`+Z`: u = X, v = Y; `+X`:
/// u = -Z; `+Y`: v = -Z).
pub fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let n = n.normalize_or(Vec3::Z);
    let up = if n.y.abs() < 0.99 {
        Vec3::Y
    } else {
        Vec3::new(0.0, 0.0, -n.y.signum())
    };
    let u = up.cross(n).normalize_or(Vec3::X);
    (u, n.cross(u))
}

/// Box (cube) projection of a point on a face with normal `normal`: the
/// dominant axis of the normal picks the plane. `scale` UV units per metre.
pub fn box_uv(p: Vec3, normal: Vec3, scale: f32) -> Vec2 {
    let a = normal.abs();
    let axis = if a.x >= a.y && a.x >= a.z {
        Vec3::X * normal.x.signum()
    } else if a.y >= a.z {
        Vec3::Y * normal.y.signum()
    } else {
        Vec3::Z
            * if normal.z == 0.0 {
                1.0
            } else {
                normal.z.signum()
            }
    };
    let (u, v) = plane_basis(axis);
    Vec2::new(p.dot(u), p.dot(v)) * scale
}

/// Signed area of a UV polygon (positive = counter-clockwise).
pub(crate) fn signed_area(uv: &[Vec2]) -> f32 {
    let n = uv.len();
    (0..n).map(|i| uv[i].perp_dot(uv[(i + 1) % n])).sum::<f32>() * 0.5
}

impl PolyMesh {
    /// Whether the mesh carries UVs (`uvs` is parallel to `faces`).
    pub fn has_uvs(&self) -> bool {
        !self.uvs.is_empty()
    }

    /// UV of vertex `v` in face `f` (its first corner), zero if absent.
    pub(crate) fn corner_uv(&self, f: usize, v: u32) -> Vec2 {
        self.faces[f]
            .iter()
            .position(|&w| w == v)
            .map_or(Vec2::ZERO, |i| self.uvs[f][i])
    }

    /// Box-projected UVs for a face loop (used for new faces and on the fly).
    pub(crate) fn auto_face_uvs(&self, face: &[u32]) -> Vec<Vec2> {
        let pts: Vec<Vec3> = face.iter().map(|&v| self.positions[v as usize]).collect();
        let normal = crate::query::newell(pts.iter().copied());
        pts.iter().map(|&p| box_uv(p, normal, 1.0)).collect()
    }

    /// UV inside face `f` at the point `p` (in or near the face plane), by
    /// mean value coordinates of the face polygon. Exact on vertices and edges.
    pub(crate) fn face_uv_at(&self, f: usize, p: Vec3) -> Vec2 {
        let face = &self.faces[f];
        let uvs = &self.uvs[f];
        let (bu, bv) = plane_basis(self.face_normal(f));
        let flat = |q: Vec3| Vec2::new(q.dot(bu), q.dot(bv));
        let x = flat(p);
        let d: Vec<Vec2> = face
            .iter()
            .map(|&v| flat(self.positions[v as usize]) - x)
            .collect();
        let n = face.len();
        let scale = d.iter().map(|v| v.length()).fold(0.0, f32::max).max(1e-12);
        let eps = scale * 1e-5;
        for i in 0..n {
            if d[i].length() <= eps {
                return uvs[i];
            }
        }
        let mut tan_half = vec![0.0; n];
        for i in 0..n {
            let (a, b) = (d[i], d[(i + 1) % n]);
            let (la, lb) = (a.length(), b.length());
            let cross = a.perp_dot(b);
            let dot = a.dot(b);
            if cross.abs() <= eps * (la + lb) && dot < 0.0 {
                // On the edge i -> i+1.
                return uvs[i].lerp(uvs[(i + 1) % n], la / (la + lb));
            }
            // tan(α/2) = |a×b| / (|a||b| + a·b), signed by the turn.
            tan_half[i] = cross / (la * lb + dot).max(1e-20);
        }
        let mut sum = 0.0;
        let mut out = Vec2::ZERO;
        for i in 0..n {
            let w = (tan_half[(i + n - 1) % n] + tan_half[i]) / d[i].length();
            sum += w;
            out += uvs[i] * w;
        }
        if sum.abs() > 1e-20 && out.is_finite() {
            out / sum
        } else {
            uvs.iter().sum::<Vec2>() / n as f32
        }
    }

    /// UVs for a side quad `[p, q, q + offset, p + offset]` built on the
    /// edge `p`-`q` (extrusion), laid out isometrically and never mirrored.
    /// A face running `q -> p` stays at the base: the wall continues its UV
    /// edge outwards. A face running `p -> q` is the extruded face itself,
    /// now at the top: the wall hangs off its UV edge. Wire edges start a
    /// fresh 1 UV/metre layout.
    pub(crate) fn wall_uvs(&self, p: u32, q: u32, offset: Vec3, skip: &[usize]) -> Vec<Vec2> {
        let (pp, pq) = (self.positions[p as usize], self.positions[q as usize]);
        let e = pq - pp;
        let len = e.length();
        let usable = |f: &usize| !skip.contains(f);
        let src = (0..self.faces.len())
            .filter(usable)
            .find(|&f| find_directed(&self.faces[f], q, p).is_some())
            .map(|f| (f, true))
            .or_else(|| {
                (0..self.faces.len())
                    .filter(usable)
                    .find(|&f| find_directed(&self.faces[f], p, q).is_some())
                    .map(|f| (f, false))
            });
        let (up, uq, side, at_base) = match src {
            Some((f, at_base)) if self.has_uvs() => {
                let side = if signed_area(&self.uvs[f]) < 0.0 {
                    -1.0
                } else {
                    1.0
                };
                (self.corner_uv(f, p), self.corner_uv(f, q), side, at_base)
            }
            _ => (Vec2::ZERO, Vec2::new(len, 0.0), 1.0, true),
        };
        let duv = uq - up;
        let (x, k) = if duv.length() > 1e-9 && len > 1e-9 {
            (duv / duv.length(), duv.length() / len)
        } else {
            (Vec2::X, 1.0)
        };
        let y = x.perp() * side;
        let ex = if len > 1e-9 { e / len } else { Vec3::X };
        let along = offset.dot(ex);
        let across = (offset - ex * along).length();
        let lift = (x * along + y * across) * k;
        if at_base {
            vec![up, uq, uq + lift, up + lift]
        } else {
            vec![up - lift, uq - lift, uq, up]
        }
    }

    // --- projections -----------------------------------------------------

    /// Gives every face UVs if the mesh has none yet (box projection, 1 UV
    /// per metre). Returns whether anything changed.
    pub fn ensure_uvs(&mut self) -> bool {
        if self.uvs.len() == self.faces.len() {
            return false;
        }
        self.uvs = (0..self.faces.len())
            .map(|f| self.auto_face_uvs(&self.faces[f]))
            .collect();
        true
    }

    /// Box (cube) projection: each face is projected along the dominant axis
    /// of its normal, `scale` UV units per metre.
    pub fn project_box(&mut self, faces: &[usize], scale: f32) {
        self.ensure_uvs();
        for f in unique_faces(self, faces) {
            let n = self.face_normal(f);
            self.uvs[f] = self.faces[f]
                .iter()
                .map(|&v| box_uv(self.positions[v as usize], n, scale))
                .collect();
        }
    }

    /// Planar projection of the faces along `normal` (`None`: their
    /// area-weighted average normal), `scale` UV units per metre, the
    /// projection's lower-left corner at the UV origin.
    pub fn project_planar(&mut self, faces: &[usize], normal: Option<Vec3>, scale: f32) {
        self.ensure_uvs();
        let faces = unique_faces(self, faces);
        let n = normal.unwrap_or_else(|| faces.iter().map(|&f| self.face_area_vector(f)).sum());
        let (u, v) = plane_basis(n);
        self.project_with(&faces, |p| Vec2::new(p.dot(u), p.dot(v)) * scale, true);
    }

    /// Writes `map(position)` into the faces' corners, optionally shifting
    /// the result so its bounds start at the UV origin.
    fn project_with(&mut self, faces: &[usize], map: impl Fn(Vec3) -> Vec2, to_origin: bool) {
        for &f in faces {
            self.uvs[f] = self.faces[f]
                .iter()
                .map(|&v| map(self.positions[v as usize]))
                .collect();
        }
        if to_origin {
            let min = faces
                .iter()
                .flat_map(|&f| self.uvs[f].iter().copied())
                .fold(Vec2::splat(f32::INFINITY), Vec2::min);
            if min.is_finite() {
                for &f in faces {
                    for t in &mut self.uvs[f] {
                        *t -= min;
                    }
                }
            }
        }
    }

    /// Centre of the faces' bounding box and the faces' vertices.
    fn faces_bounds_center(&self, faces: &[usize]) -> Vec3 {
        let (lo, hi) = faces
            .iter()
            .flat_map(|&f| self.faces[f].iter())
            .map(|&v| self.positions[v as usize])
            .fold(
                (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                |(lo, hi), p| (lo.min(p), hi.max(p)),
            );
        if lo.is_finite() {
            (lo + hi) * 0.5
        } else {
            Vec3::ZERO
        }
    }

    /// Cylinder projection around `axis` through the faces' bounding-box
    /// centre: U is the angle (0..1, seam behind the `-u` basis direction),
    /// V the height normalized to 0..1 over the faces.
    pub fn project_cylinder(&mut self, faces: &[usize], axis: Vec3) {
        self.ensure_uvs();
        let faces = unique_faces(self, faces);
        let axis = axis.normalize_or(Vec3::Y);
        let c = self.faces_bounds_center(&faces);
        let (bu, bv) = plane_basis(axis);
        let heights = faces
            .iter()
            .flat_map(|&f| self.faces[f].iter())
            .map(|&v| (self.positions[v as usize] - c).dot(axis));
        let (lo, hi) = heights.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), h| {
            (lo.min(h), hi.max(h))
        });
        let span = if hi - lo > 1e-9 { hi - lo } else { 1.0 };
        self.project_round(&faces, |p| {
            let d = p - c;
            let r = Vec2::new(d.dot(bu), d.dot(bv));
            let u = (r.length() > 1e-6).then(|| r.y.atan2(r.x) / TAU + 0.5);
            (u, (d.dot(axis) - lo) / span)
        });
    }

    /// Sphere projection around `axis` through the faces' bounding-box
    /// centre: U longitude, V latitude, both 0..1.
    pub fn project_sphere(&mut self, faces: &[usize], axis: Vec3) {
        self.ensure_uvs();
        let faces = unique_faces(self, faces);
        let axis = axis.normalize_or(Vec3::Y);
        let c = self.faces_bounds_center(&faces);
        let (bu, bv) = plane_basis(axis);
        self.project_round(&faces, |p| {
            let d = p - c;
            let r = Vec2::new(d.dot(bu), d.dot(bv));
            let u = (r.length() > 1e-6 * d.length().max(1e-6)).then(|| r.y.atan2(r.x) / TAU + 0.5);
            let lat = if d.length() > 1e-9 {
                (d.dot(axis) / d.length()).clamp(-1.0, 1.0).asin()
            } else {
                0.0
            };
            (u, lat / PI + 0.5)
        });
    }

    /// Shared by cylinder and sphere: `map` gives `(u or None at a pole, v)`.
    /// Faces crossing the wrap-around get their low U values moved past 1;
    /// pole corners take the mean U of the face's other corners.
    fn project_round(&mut self, faces: &[usize], map: impl Fn(Vec3) -> (Option<f32>, f32)) {
        for &f in faces {
            let raw: Vec<(Option<f32>, f32)> = self.faces[f]
                .iter()
                .map(|&v| map(self.positions[v as usize]))
                .collect();
            let mut us: Vec<f32> = raw.iter().filter_map(|r| r.0).collect();
            let (lo, hi) = us
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &u| {
                    (a.min(u), b.max(u))
                });
            let wrap = hi - lo > 0.5;
            let fix = |u: f32| if wrap && u < 0.5 { u + 1.0 } else { u };
            for u in &mut us {
                *u = fix(*u);
            }
            let mean = if us.is_empty() {
                0.5
            } else {
                us.iter().sum::<f32>() / us.len() as f32
            };
            self.uvs[f] = raw
                .iter()
                .map(|&(u, v)| Vec2::new(u.map_or(mean, fix), v))
                .collect();
        }
    }

    /// Projects the faces through `view_proj` (mesh space -> clip space,
    /// i.e. `projection * view * model`): the view's rectangle becomes 0..1.
    pub fn project_from_view(&mut self, faces: &[usize], view_proj: Mat4) {
        self.ensure_uvs();
        let faces = unique_faces(self, faces);
        self.project_with(
            &faces,
            |p| {
                let ndc = view_proj.project_point3(p);
                let uv = Vec2::new(ndc.x, ndc.y) * 0.5 + 0.5;
                if uv.is_finite() { uv } else { Vec2::ZERO }
            },
            false,
        );
    }

    /// Blender's UV "Reset": every face fills the whole 0..1 square (quads
    /// corner to corner, other polygons inscribed in it).
    pub fn reset_uvs(&mut self, faces: &[usize]) {
        self.ensure_uvs();
        for f in unique_faces(self, faces) {
            let n = self.faces[f].len();
            self.uvs[f] = if n == 4 {
                vec![
                    Vec2::new(0.0, 0.0),
                    Vec2::new(1.0, 0.0),
                    Vec2::new(1.0, 1.0),
                    Vec2::new(0.0, 1.0),
                ]
            } else {
                (0..n)
                    .map(|i| {
                        let a = -PI / 2.0 - PI / n as f32 + TAU * i as f32 / n as f32;
                        Vec2::new(a.cos(), a.sin()) * 0.5 + 0.5
                    })
                    .collect()
            };
        }
    }

    /// Lays quads out as a grid continuing from `active` (Blender's "Follow
    /// Active Quads", length-average mode): each quad reached across an edge
    /// becomes a rectangle-ish continuation of its neighbour. Only quads in
    /// `faces` change. Returns how many faces were laid out (active included).
    pub fn follow_active_quads(&mut self, faces: &[usize], active: usize) -> usize {
        self.ensure_uvs();
        let set: HashSet<usize> = unique_faces(self, faces)
            .into_iter()
            .filter(|&f| self.faces[f].len() == 4)
            .collect();
        if !set.contains(&active) {
            return 0;
        }
        let adj = self.adjacency();
        let mut done = HashSet::from([active]);
        let mut queue = std::collections::VecDeque::from([active]);
        while let Some(f) = queue.pop_front() {
            let sign = if signed_area(&self.uvs[f]) < 0.0 {
                -1.0
            } else {
                1.0
            };
            let loop_f = self.faces[f].clone();
            for (a, b) in loop_edges(&loop_f) {
                for &g in adj.faces(a, b) {
                    if !set.contains(&g) || done.contains(&g) {
                        continue;
                    }
                    let (ua, ub) = (self.corner_uv(f, a), self.corner_uv(f, b));
                    let quad = &self.faces[g];
                    // Rotate g to start at the shared edge.
                    let (k, flip) = match find_directed(quad, b, a) {
                        Some(k) => (k, false),
                        None => match find_directed(quad, a, b) {
                            Some(k) => (k, true),
                            None => continue,
                        },
                    };
                    let at = |i: usize| quad[(k + i) % 4];
                    let (x0, x1, x2, x3) = (at(0), at(1), at(2), at(3));
                    let p = |v: u32| self.positions[v as usize];
                    // x0-x1 is the shared edge; x2 hangs off x1, x3 off x0.
                    let (u0, u1) = if flip { (ua, ub) } else { (ub, ua) };
                    let edge3 = p(x1).distance(p(x0));
                    let side3 = (p(x2).distance(p(x1)) + p(x3).distance(p(x0))) * 0.5;
                    let duv = u1 - u0;
                    let ratio = if edge3 > 1e-9 {
                        duv.length() / edge3
                    } else {
                        1.0
                    };
                    // Away from f: f lies on the left of a -> b when CCW.
                    let dir = duv.normalize_or(Vec2::X);
                    let away = if flip { -dir.perp() } else { dir.perp() } * sign;
                    let lift = away * side3 * ratio;
                    let corners = [u0, u1, u1 + lift, u0 + lift];
                    let order = [x0, x1, x2, x3];
                    let mut uv = vec![Vec2::ZERO; 4];
                    for (i, slot) in uv.iter_mut().enumerate() {
                        let v = self.faces[g][i];
                        *slot = corners[order.iter().position(|&o| o == v).unwrap_or(0)];
                    }
                    self.uvs[g] = uv;
                    done.insert(g);
                    queue.push_back(g);
                }
            }
        }
        done.len()
    }

    // --- seams, islands, transforms -------------------------------------

    /// Marks (`on`) or clears UV seams on existing edges.
    pub fn mark_seam(&mut self, edges: &[[u32; 2]], on: bool) {
        let exists: HashSet<[u32; 2]> = self.edges().into_iter().collect();
        for &[a, b] in edges {
            let key = edge_key(a, b);
            if on {
                if a != b && exists.contains(&key) && !self.seams.contains(&key) {
                    self.seams.push(key);
                }
            } else {
                self.seams.retain(|&s| s != key);
            }
        }
    }

    pub fn is_seam(&self, a: u32, b: u32) -> bool {
        self.seams.contains(&edge_key(a, b))
    }

    /// UV islands: faces joined across edges whose two endpoints have the
    /// same UVs on both sides. Each island is sorted; islands are ordered
    /// by their first face. Without UVs every connected patch of the box
    /// projection counts, which is simply each face.
    pub fn islands(&self) -> Vec<Vec<usize>> {
        let n = self.faces.len();
        if !self.has_uvs() {
            return (0..n).map(|f| vec![f]).collect();
        }
        let mut parent: Vec<usize> = (0..n).collect();
        let adj = self.adjacency();
        for (&[a, b], fs) in &adj.edge_faces {
            for i in 0..fs.len() {
                for j in i + 1..fs.len() {
                    let (f, g) = (fs[i], fs[j]);
                    let same =
                        |v: u32| self.corner_uv(f, v).distance(self.corner_uv(g, v)) <= UV_EPS;
                    if same(a) && same(b) {
                        union(&mut parent, f, g);
                    }
                }
            }
        }
        groups(&mut parent)
    }

    /// Applies `m` to the given corners' UVs (each listed corner once).
    /// Out-of-range corners are ignored.
    pub fn transform_uvs(&mut self, corners: &[UvCorner], m: Affine2) {
        self.ensure_uvs();
        let mut seen = HashSet::new();
        for &(f, i) in corners {
            if seen.insert((f, i))
                && let Some(t) = self.uvs.get_mut(f).and_then(|u| u.get_mut(i))
            {
                *t = m.transform_point2(*t);
            }
        }
    }

    /// Every corner of the given faces.
    pub fn face_corners(&self, faces: &[usize]) -> Vec<UvCorner> {
        unique_faces(self, faces)
            .into_iter()
            .flat_map(|f| (0..self.faces[f].len()).map(move |i| (f, i)))
            .collect()
    }

    /// Texel density of the faces in pixels per metre for a square texture
    /// of `texture_px` pixels: `texture_px * sqrt(uv area / surface area)`.
    /// `None` without UVs or area.
    pub fn texel_density(&self, faces: &[usize], texture_px: f32) -> Option<f32> {
        if !self.has_uvs() {
            return None;
        }
        let faces = unique_faces(self, faces);
        let uv: f32 = faces.iter().map(|&f| signed_area(&self.uvs[f]).abs()).sum();
        let area: f32 = faces
            .iter()
            .map(|&f| self.face_area_vector(f).length())
            .sum();
        (area > 1e-12 && uv > 0.0).then(|| texture_px * (uv / area).sqrt())
    }

    /// Flat-shaded render data like [`PolyMesh::triangulate`], plus one UV
    /// per corner (as stored: origin bottom-left, V up — flip V when
    /// sampling a top-left-origin texture). Meshes without UVs get a box
    /// projection on the fly.
    pub fn triangulate_uv(&self) -> (Vec<Vec3>, Vec<Vec3>, Vec<Vec2>, Vec<u32>) {
        let (positions, normals, indices) = self.triangulate();
        let mut uvs = Vec::with_capacity(positions.len());
        for (f, face) in self.faces.iter().enumerate() {
            if self.has_uvs() {
                uvs.extend_from_slice(&self.uvs[f]);
            } else {
                uvs.extend(self.auto_face_uvs(face));
            }
        }
        (positions, normals, uvs, indices)
    }
}

pub(crate) fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

pub(crate) fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra.max(rb)] = ra.min(rb);
    }
}

/// Union-find groups, each sorted, ordered by smallest member.
pub(crate) fn groups(parent: &mut [usize]) -> Vec<Vec<usize>> {
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..parent.len() {
        let r = find(parent, i);
        by_root.entry(r).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = by_root.into_values().collect();
    out.sort_unstable_by_key(|g| g[0]);
    out
}
