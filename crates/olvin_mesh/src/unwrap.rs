//! Real UV unwrapping: faces are cut into charts at seams, every disk-like
//! chart is flattened with LSCM (least squares conformal maps, Lévy et al.
//! 2002) solved by preconditioned conjugate gradients; closed or holed
//! charts are split by dominant normal axis first. The charts are scaled to
//! their surface area, rotated to their tightest bounding box and shelf
//! packed into 0..1.

use std::collections::{HashMap, HashSet};

use glam::{DVec2, Vec2, Vec3};

use crate::extrude::unique_faces;
use crate::uv::{groups, plane_basis, signed_area, union};
use crate::{PolyMesh, edge_key, loop_edges};

/// Faces flattened together; corners of one vertex that stay joined across
/// unsplit edges share a chart vertex.
struct Chart {
    faces: Vec<usize>,
    /// `(face, corner)` -> chart vertex.
    corner: HashMap<(usize, usize), usize>,
    /// 3D position of each chart vertex.
    points: Vec<Vec3>,
}

impl Chart {
    /// Euler characteristic V - E + F (1 for a disk).
    fn euler(&self, mesh: &PolyMesh) -> i64 {
        let mut edges = HashSet::new();
        for &f in &self.faces {
            let n = mesh.faces[f].len();
            for i in 0..n {
                let (a, b) = (self.corner[&(f, i)], self.corner[&(f, (i + 1) % n)]);
                edges.insert((a.min(b), a.max(b)));
            }
        }
        self.points.len() as i64 - edges.len() as i64 + self.faces.len() as i64
    }
}

/// Dominant signed axis of a normal, 0..6.
fn axis_class(n: Vec3) -> u8 {
    let a = n.abs();
    let (axis, s) = if a.x >= a.y && a.x >= a.z {
        (0, n.x)
    } else if a.y >= a.z {
        (1, n.y)
    } else {
        (2, n.z)
    };
    axis * 2 + u8::from(s < 0.0)
}

impl PolyMesh {
    /// Unwraps the faces (Blender `U` → Unwrap): charts split at seams are
    /// flattened by LSCM, scaled to their true area and packed into 0..1
    /// with `margin` between islands. Other faces keep their UVs. Returns
    /// the number of islands made.
    pub fn unwrap_faces(&mut self, faces: &[usize], margin: f32) -> usize {
        self.ensure_uvs();
        let faces = unique_faces(self, faces);
        if faces.is_empty() {
            return 0;
        }
        let seams: HashSet<[u32; 2]> = self.seams.iter().copied().collect();
        let mut done: Vec<Vec<usize>> = Vec::new();
        for chart in self.charts(&faces, |_, _, e| !seams.contains(&e)) {
            if chart.euler(self) == 1 {
                self.flatten(&chart);
                done.push(chart.faces);
                continue;
            }
            // Closed (or holed) surface without enough seams: cut it where
            // the dominant normal axis changes, like a box projection.
            let class: HashMap<usize, u8> = chart
                .faces
                .iter()
                .map(|&f| (f, axis_class(self.face_normal(f))))
                .collect();
            let parts = self.charts(&chart.faces, |f, g, e| {
                !seams.contains(&e) && class[&f] == class[&g]
            });
            for part in parts {
                if part.euler(self) == 1 {
                    self.flatten(&part);
                } else {
                    self.flatten_planar(&part);
                }
                done.push(part.faces);
            }
        }
        for island in &done {
            self.normalize_island(island);
        }
        self.pack_groups(&done, margin);
        done.len()
    }

    /// Packs every UV island into 0..1 keeping their relative sizes,
    /// `margin` apart and from the border.
    pub fn pack_islands(&mut self, margin: f32) {
        self.ensure_uvs();
        let all = self.islands();
        self.pack_groups(&all, margin);
    }

    /// Packs only the islands touching `faces` into 0..1.
    pub fn pack_face_islands(&mut self, faces: &[usize], margin: f32) {
        self.ensure_uvs();
        let wanted: HashSet<usize> = unique_faces(self, faces).into_iter().collect();
        let picked: Vec<Vec<usize>> = self
            .islands()
            .into_iter()
            .filter(|i| i.iter().any(|f| wanted.contains(f)))
            .collect();
        self.pack_groups(&picked, margin);
    }

    /// Splits `faces` into charts: faces join across an edge when
    /// `joined(f, g, edge)` holds, corners of a vertex join likewise.
    fn charts(
        &self,
        faces: &[usize],
        joined: impl Fn(usize, usize, [u32; 2]) -> bool,
    ) -> Vec<Chart> {
        let index: HashMap<usize, usize> = faces.iter().enumerate().map(|(i, &f)| (f, i)).collect();
        let mut first_corner = Vec::with_capacity(faces.len());
        let mut total = 0;
        for &f in faces {
            first_corner.push(total);
            total += self.faces[f].len();
        }
        let corner_id = |f: usize, v: u32| -> Option<usize> {
            let i = self.faces[f].iter().position(|&w| w == v)?;
            Some(first_corner[index[&f]] + i)
        };
        let mut face_parent: Vec<usize> = (0..faces.len()).collect();
        let mut corner_parent: Vec<usize> = (0..total).collect();
        let mut edge_faces: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        for &f in faces {
            for (a, b) in loop_edges(&self.faces[f]) {
                edge_faces.entry(edge_key(a, b)).or_default().push(f);
            }
        }
        let mut keys: Vec<&[u32; 2]> = edge_faces.keys().collect();
        keys.sort_unstable();
        for key in keys {
            let fs = &edge_faces[key];
            for i in 0..fs.len() {
                for j in i + 1..fs.len() {
                    let (f, g) = (fs[i], fs[j]);
                    if f == g || !joined(f, g, *key) {
                        continue;
                    }
                    union(&mut face_parent, index[&f], index[&g]);
                    for v in *key {
                        if let (Some(x), Some(y)) = (corner_id(f, v), corner_id(g, v)) {
                            union(&mut corner_parent, x, y);
                        }
                    }
                }
            }
        }
        groups(&mut face_parent)
            .into_iter()
            .map(|group| {
                let mut chart = Chart {
                    faces: group.iter().map(|&i| faces[i]).collect(),
                    corner: HashMap::new(),
                    points: Vec::new(),
                };
                let mut local: HashMap<usize, usize> = HashMap::new();
                for &i in &group {
                    let f = faces[i];
                    for (k, &v) in self.faces[f].iter().enumerate() {
                        let root = crate::uv::find(&mut corner_parent, first_corner[i] + k);
                        let id = *local.entry(root).or_insert_with(|| {
                            chart.points.push(self.positions[v as usize]);
                            chart.points.len() - 1
                        });
                        chart.corner.insert((f, k), id);
                    }
                }
                chart
            })
            .collect()
    }

    fn write_chart(&mut self, chart: &Chart, uv: &[Vec2]) {
        for &f in &chart.faces {
            self.uvs[f] = (0..self.faces[f].len())
                .map(|i| uv[chart.corner[&(f, i)]])
                .collect();
        }
    }

    fn flatten_planar(&mut self, chart: &Chart) {
        let n: Vec3 = chart.faces.iter().map(|&f| self.face_area_vector(f)).sum();
        let (u, v) = plane_basis(n);
        let uv: Vec<Vec2> = chart
            .points
            .iter()
            .map(|p| Vec2::new(p.dot(u), p.dot(v)))
            .collect();
        self.write_chart(chart, &uv);
    }

    /// LSCM with two pinned vertices; falls back to planar on failure.
    fn flatten(&mut self, chart: &Chart) {
        match self.lscm(chart) {
            Some(uv) => self.write_chart(chart, &uv),
            None => self.flatten_planar(chart),
        }
    }

    fn lscm(&self, chart: &Chart) -> Option<Vec<Vec2>> {
        let pts = &chart.points;
        let nv = pts.len();
        if nv < 3 {
            return None;
        }
        // Pins: the extremes along the longest bounding-box axis.
        let (lo, hi) = pts.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let ext = hi - lo;
        let dir = if ext.x >= ext.y && ext.x >= ext.z {
            Vec3::X
        } else if ext.y >= ext.z {
            Vec3::Y
        } else {
            Vec3::Z
        };
        let key = |i: &usize| pts[*i].dot(dir);
        let p0 = (0..nv).min_by(|a, b| key(a).total_cmp(&key(b)))?;
        let p1 = (0..nv).max_by(|a, b| key(a).total_cmp(&key(b)))?;
        if p0 == p1 || pts[p0].distance(pts[p1]) < 1e-9 {
            return None;
        }

        // Initial guess: planar projection, moved by a similarity so the
        // pins land on (0, 0) and (|p1 - p0|, 0).
        let normal: Vec3 = chart.faces.iter().map(|&f| self.face_area_vector(f)).sum();
        let (bu, bv) = plane_basis(normal);
        let planar: Vec<DVec2> = pts
            .iter()
            .map(|p| DVec2::new(p.dot(bu) as f64, p.dot(bv) as f64))
            .collect();
        let target = DVec2::new(pts[p0].distance(pts[p1]) as f64, 0.0);
        let span = planar[p1] - planar[p0];
        let guess: Vec<DVec2> = if span.length() > 1e-12 {
            // z -> (z - z0) * target / span, as complex numbers.
            let rot = DVec2::new(target.x, 0.0).rotate(DVec2::new(span.x, -span.y))
                / span.length_squared();
            planar.iter().map(|&z| rot.rotate(z - planar[p0])).collect()
        } else {
            planar.iter().map(|&z| z - planar[p0]).collect()
        };
        let pinned = |i: usize| match i {
            _ if i == p0 => Some(DVec2::ZERO),
            _ if i == p1 => Some(target),
            _ => None,
        };
        let mut col = vec![usize::MAX; nv];
        let mut free = 0;
        for (i, c) in col.iter_mut().enumerate() {
            if pinned(i).is_none() {
                *c = free;
                free += 1;
            }
        }
        let unknowns = free * 2;

        // Two rows per triangle: Σ W_j (u_j + i v_j) = 0, W_j = x_{j+2} - x_{j+1}.
        let mut rows: Vec<Vec<(usize, f64)>> = Vec::new();
        let mut rhs: Vec<f64> = Vec::new();
        for &f in &chart.faces {
            for tri in self.face_triangles_local(f) {
                let ids = tri.map(|i| chart.corner[&(f, i as usize)]);
                let p = ids.map(|i| pts[i].as_dvec3());
                let e1 = p[1] - p[0];
                let n = e1.cross(p[2] - p[0]);
                let area2 = n.length();
                if area2 < 1e-14 || e1.length() < 1e-12 {
                    continue;
                }
                let ex = e1.normalize();
                let ey = n.cross(ex).normalize();
                let x = p.map(|q| DVec2::new((q - p[0]).dot(ex), (q - p[0]).dot(ey)));
                let w = 1.0 / area2.sqrt();
                let mut re = Vec::with_capacity(6);
                let mut im = Vec::with_capacity(6);
                let (mut b_re, mut b_im) = (0.0, 0.0);
                for j in 0..3 {
                    let wj = (x[(j + 2) % 3] - x[(j + 1) % 3]) * w;
                    let (a, b) = (wj.x, wj.y);
                    match pinned(ids[j]) {
                        Some(u) => {
                            b_re -= a * u.x - b * u.y;
                            b_im -= b * u.x + a * u.y;
                        }
                        None => {
                            let c = col[ids[j]];
                            re.extend([(2 * c, a), (2 * c + 1, -b)]);
                            im.extend([(2 * c, b), (2 * c + 1, a)]);
                        }
                    }
                }
                rows.extend([re, im]);
                rhs.extend([b_re, b_im]);
            }
        }
        let mut x0 = vec![0.0; unknowns];
        for i in 0..nv {
            if col[i] != usize::MAX {
                x0[2 * col[i]] = guess[i].x;
                x0[2 * col[i] + 1] = guess[i].y;
            }
        }
        let sol = cgls(&rows, &rhs, x0, unknowns)?;
        let mut uv: Vec<Vec2> = (0..nv)
            .map(|i| match pinned(i) {
                Some(p) => p.as_vec2(),
                None => DVec2::new(sol[2 * col[i]], sol[2 * col[i] + 1]).as_vec2(),
            })
            .collect();
        if uv.iter().any(|t| !t.is_finite()) {
            return None;
        }
        // Mirror if the solution came out flipped (it should not).
        let area: f32 = chart
            .faces
            .iter()
            .map(|&f| {
                let loop_uv: Vec<Vec2> = (0..self.faces[f].len())
                    .map(|i| uv[chart.corner[&(f, i)]])
                    .collect();
                signed_area(&loop_uv)
            })
            .sum();
        if area.abs() < 1e-12 {
            return None;
        }
        if area < 0.0 {
            for t in &mut uv {
                t.x = -t.x;
            }
        }
        Some(uv)
    }

    /// Scales an island so its UV area equals its surface area, then turns
    /// it to its minimum-area bounding box, lying flat (wider than tall).
    fn normalize_island(&mut self, faces: &[usize]) {
        let uv_area: f32 = faces.iter().map(|&f| signed_area(&self.uvs[f]).abs()).sum();
        let area: f32 = faces
            .iter()
            .map(|&f| self.face_area_vector(f).length())
            .sum();
        let scale = if uv_area > 1e-12 && area > 1e-12 {
            (area / uv_area).sqrt()
        } else {
            1.0
        };
        let pts: Vec<Vec2> = faces
            .iter()
            .flat_map(|&f| self.uvs[f].iter().copied())
            .collect();
        let hull = convex_hull(&pts);
        let mut best = (f32::INFINITY, Vec2::X);
        for i in 0..hull.len() {
            let d = (hull[(i + 1) % hull.len()] - hull[i]).normalize_or_zero();
            if d == Vec2::ZERO {
                continue;
            }
            // Rotate by -angle(d): x' = d·p, y' = d⊥·p.
            let (lo, hi) = bounds(hull.iter().map(|&p| Vec2::new(d.dot(p), d.perp_dot(p))));
            let size = hi - lo;
            if size.x * size.y < best.0 - 1e-9 {
                best = (size.x * size.y, d);
            }
        }
        let d = best.1;
        let (lo, hi) = bounds(pts.iter().map(|&p| Vec2::new(d.dot(p), d.perp_dot(p))));
        let size = hi - lo;
        let tall = size.y > size.x * 1.0001;
        for &f in faces {
            for t in &mut self.uvs[f] {
                let mut r = Vec2::new(d.dot(*t), d.perp_dot(*t)) - lo;
                if tall {
                    // Quarter turn keeps orientation: (x, y) -> (y, -x) + (0, w).
                    r = Vec2::new(r.y, size.x - r.x);
                }
                *t = r * scale;
            }
        }
    }

    /// Shelf-packs the groups' bounding boxes into 0..1 with a uniform scale.
    pub(crate) fn pack_groups(&mut self, groups: &[Vec<usize>], margin: f32) {
        if groups.is_empty() {
            return;
        }
        let margin = margin.clamp(0.0, 0.25);
        let boxes: Vec<(Vec2, Vec2)> = groups
            .iter()
            .map(|g| bounds(g.iter().flat_map(|&f| self.uvs[f].iter().copied())))
            .collect();
        let sizes: Vec<Vec2> = boxes
            .iter()
            .map(|(lo, hi)| (*hi - *lo).max(Vec2::ZERO))
            .collect();
        let total: f32 = sizes.iter().map(|s| s.x * s.y).sum();
        let largest = sizes.iter().fold(0.0f32, |m, s| m.max(s.x).max(s.y));
        if largest <= 0.0 {
            return;
        }
        let mut scale = 1.0 / total.sqrt().max(largest);
        let mut layout = (Vec::new(), 1.0);
        for _ in 0..6 {
            let gap = margin / scale;
            layout = best_shelf(&sizes, gap);
            scale = 1.0 / layout.1;
        }
        let (places, extent) = layout;
        let scale = 1.0 / extent;
        for ((g, (lo, _)), place) in groups.iter().zip(&boxes).zip(&places) {
            for &f in g {
                for t in &mut self.uvs[f] {
                    *t = (*t - *lo + *place) * scale;
                }
            }
        }
    }
}

/// Tries a few row widths and keeps the squarest packing: positions and the
/// side of the square it fits in.
fn best_shelf(sizes: &[Vec2], gap: f32) -> (Vec<Vec2>, f32) {
    let area: f32 = sizes.iter().map(|s| (s.x + gap) * (s.y + gap)).sum();
    let widest = sizes.iter().fold(0.0f32, |m, s| m.max(s.x)) + 2.0 * gap;
    let mut best: Option<(Vec<Vec2>, f32)> = None;
    for k in 0..=12 {
        let width = (area.sqrt() * (0.8 + 0.06 * k as f32)).max(widest);
        let (places, extent) = shelf(sizes, gap, width);
        let side = extent.x.max(extent.y);
        if best.as_ref().is_none_or(|b| side < b.1) {
            best = Some((places, side));
        }
    }
    best.unwrap_or_default()
}

/// Rows of boxes sorted by height, `gap` between them and around the edge.
fn shelf(sizes: &[Vec2], gap: f32, width: f32) -> (Vec<Vec2>, Vec2) {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|&a, &b| sizes[b].y.total_cmp(&sizes[a].y).then(a.cmp(&b)));
    let mut places = vec![Vec2::ZERO; sizes.len()];
    let (mut x, mut y, mut row, mut used) = (gap, gap, 0.0f32, 0.0f32);
    for i in order {
        let s = sizes[i];
        if x > gap && x + s.x + gap > width {
            y += row + gap;
            x = gap;
            row = 0.0;
        }
        places[i] = Vec2::new(x, y);
        x += s.x + gap;
        row = row.max(s.y);
        used = used.max(x);
    }
    (places, Vec2::new(used, y + row + gap))
}

fn bounds(points: impl Iterator<Item = Vec2>) -> (Vec2, Vec2) {
    let (lo, hi) = points.fold(
        (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    if lo.is_finite() && hi.is_finite() {
        (lo, hi)
    } else {
        (Vec2::ZERO, Vec2::ZERO)
    }
}

/// Andrew's monotone chain, counter-clockwise, no collinear points.
fn convex_hull(points: &[Vec2]) -> Vec<Vec2> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut hull: Vec<Vec2> = Vec::with_capacity(pts.len() * 2);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 {
            Box::new(pts.iter())
        } else {
            Box::new(pts.iter().rev())
        };
        for &p in iter {
            while hull.len() >= start + 2 {
                let (a, b) = (hull[hull.len() - 2], hull[hull.len() - 1]);
                if (b - a).perp_dot(p - b) <= 0.0 {
                    hull.pop();
                } else {
                    break;
                }
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// Least squares `min |A x - b|` by conjugate gradients on the normal
/// equations (CGLS) with Jacobi column scaling.
fn cgls(rows: &[Vec<(usize, f64)>], b: &[f64], x0: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    if n == 0 {
        return Some(x0);
    }
    let mut norm = vec![0.0f64; n];
    for row in rows {
        for &(c, v) in row {
            norm[c] += v * v;
        }
    }
    let d: Vec<f64> = norm
        .iter()
        .map(|&s| if s > 1e-30 { 1.0 / s.sqrt() } else { 0.0 })
        .collect();
    // Solve for y with x = D y.
    let mul = |y: &[f64]| -> Vec<f64> {
        rows.iter()
            .map(|row| row.iter().map(|&(c, v)| v * d[c] * y[c]).sum())
            .collect()
    };
    let mul_t = |r: &[f64]| -> Vec<f64> {
        let mut out = vec![0.0; n];
        for (row, &ri) in rows.iter().zip(r) {
            for &(c, v) in row {
                out[c] += v * d[c] * ri;
            }
        }
        out
    };
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let mut y: Vec<f64> = x0
        .iter()
        .zip(&d)
        .map(|(&x, &di)| if di > 0.0 { x / di } else { 0.0 })
        .collect();
    let ay = mul(&y);
    let mut r: Vec<f64> = b.iter().zip(&ay).map(|(bi, ai)| bi - ai).collect();
    let mut s = mul_t(&r);
    let mut p = s.clone();
    let mut gamma = dot(&s, &s);
    let atb = mul_t(b);
    let reference = dot(&atb, &atb).max(gamma).max(1e-300);
    let max_iter = (n * 4).clamp(200, 20_000);
    for _ in 0..max_iter {
        if gamma <= reference * 1e-22 {
            break;
        }
        let q = mul(&p);
        let qq = dot(&q, &q);
        if qq <= 1e-300 {
            break;
        }
        let alpha = gamma / qq;
        for i in 0..n {
            y[i] += alpha * p[i];
        }
        for (ri, qi) in r.iter_mut().zip(&q) {
            *ri -= alpha * qi;
        }
        s = mul_t(&r);
        let next = dot(&s, &s);
        let beta = next / gamma;
        gamma = next;
        for i in 0..n {
            p[i] = s[i] + beta * p[i];
        }
    }
    let x: Vec<f64> = y.iter().zip(&d).map(|(yi, di)| yi * di).collect();
    x.iter().all(|v| v.is_finite()).then_some(x)
}
