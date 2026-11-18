//! Screen-space selection helpers: box select inclusion and occlusion of
//! the cage by the mesh itself (X-ray off).

use olvin_editor::egui::{Pos2, Rect};
use olvin_editor::ext::Ray;
use olvin_editor::glam::Vec3;

/// How a click or box changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    Replace,
    Add,
    Subtract,
}

/// Applies `picked` to a vertex selection.
pub fn combine(selection: &mut Vec<u32>, picked: &[u32], how: Combine) {
    match how {
        Combine::Replace => *selection = picked.to_vec(),
        Combine::Subtract => selection.retain(|v| !picked.contains(v)),
        Combine::Add => {
            for &v in picked {
                if !selection.contains(&v) {
                    selection.push(v);
                }
            }
        }
    }
}

/// Whether the segment `a`-`b` touches the rectangle.
pub fn segment_hits_rect(a: Pos2, b: Pos2, r: Rect) -> bool {
    if r.contains(a) || r.contains(b) {
        return true;
    }
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ];
    (0..4).any(|i| segments_cross(a, b, corners[i], corners[(i + 1) % 4]))
}

fn segments_cross(a: Pos2, b: Pos2, c: Pos2, d: Pos2) -> bool {
    let side = |p: Pos2, q: Pos2, r: Pos2| (q - p).x * (r - p).y - (q - p).y * (r - p).x;
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Box-selected edges, Blender style: those fully inside, or if there are
/// none, those crossing the box. `edges` holds screen endpoints.
pub fn box_edges(edges: &[Option<(Pos2, Pos2)>], r: Rect) -> Vec<usize> {
    let inside: Vec<usize> = (0..edges.len())
        .filter(|&i| edges[i].is_some_and(|(a, b)| r.contains(a) && r.contains(b)))
        .collect();
    if !inside.is_empty() {
        return inside;
    }
    (0..edges.len())
        .filter(|&i| edges[i].is_some_and(|(a, b)| segment_hits_rect(a, b, r)))
        .collect()
}

/// Triangles binned on a screen grid, so occlusion tests only look at the
/// triangles overlapping a point on screen.
pub struct Occluders {
    tris: Vec<[Vec3; 3]>,
    rect: Rect,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
    /// Triangles crossing the near plane: tested for every point.
    everywhere: Vec<u32>,
}

const CELL: f32 = 32.0;

impl Occluders {
    /// `screen` is the projection of each triangle (`None` if any corner is
    /// behind the camera).
    pub fn new(tris: Vec<[Vec3; 3]>, screen: &[Option<[Pos2; 3]>], rect: Rect) -> Self {
        let cols = (rect.width() / CELL).ceil().max(1.0) as usize;
        let rows = (rect.height() / CELL).ceil().max(1.0) as usize;
        let mut cells = vec![Vec::new(); cols * rows];
        let mut everywhere = Vec::new();
        for (i, s) in screen.iter().enumerate() {
            let Some(s) = s else {
                everywhere.push(i as u32);
                continue;
            };
            let bb = Rect::from_points(s);
            if !bb.intersects(rect) {
                continue;
            }
            let cell = |p: Pos2| {
                (
                    (((p.x - rect.left()) / CELL).floor().max(0.0) as usize).min(cols - 1),
                    (((p.y - rect.top()) / CELL).floor().max(0.0) as usize).min(rows - 1),
                )
            };
            let ((x0, y0), (x1, y1)) = (cell(bb.min), cell(bb.max));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    cells[y * cols + x].push(i as u32);
                }
            }
        }
        Self {
            tris,
            rect,
            cols,
            rows,
            cells,
            everywhere,
        }
    }

    /// Whether a triangle lies between `origin` and `p` (`at` = `p` on screen).
    pub fn occluded(&self, origin: Vec3, p: Vec3, at: Pos2) -> bool {
        let to = p - origin;
        let dist = to.length();
        if dist < 1e-6 {
            return false;
        }
        let ray = Ray {
            origin,
            dir: to / dist,
        };
        let limit = dist - (1e-3 + dist * 1e-4);
        let hits = |i: &u32| {
            let [a, b, c] = self.tris[*i as usize];
            ray.triangle(a, b, c).is_some_and(|t| t < limit)
        };
        if self.everywhere.iter().any(hits) {
            return true;
        }
        if !self.rect.contains(at) {
            return false;
        }
        let x = (((at.x - self.rect.left()) / CELL) as usize).min(self.cols - 1);
        let y = (((at.y - self.rect.top()) / CELL) as usize).min(self.rows - 1);
        self.cells[y * self.cols + x].iter().any(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use olvin_editor::egui::pos2;

    #[test]
    fn combine_modes() {
        let mut s = vec![1, 2];
        combine(&mut s, &[2, 3], Combine::Add);
        assert_eq!(s, [1, 2, 3]);
        combine(&mut s, &[1, 3], Combine::Subtract);
        assert_eq!(s, [2]);
        combine(&mut s, &[7], Combine::Replace);
        assert_eq!(s, [7]);
    }

    #[test]
    fn box_select_edges() {
        let r = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0));
        let inside = Some((pos2(1.0, 1.0), pos2(5.0, 5.0)));
        let crossing = Some((pos2(-5.0, 5.0), pos2(15.0, 5.0)));
        let outside = Some((pos2(20.0, 0.0), pos2(30.0, 10.0)));
        assert_eq!(box_edges(&[inside, crossing, outside, None], r), [0]);
        assert_eq!(box_edges(&[crossing, outside], r), [0]);
        assert!(segment_hits_rect(pos2(-1.0, 5.0), pos2(5.0, -1.0), r));
        assert!(!segment_hits_rect(pos2(-1.0, 1.0), pos2(1.0, -1.0), r));
    }

    #[test]
    fn occlusion_by_triangles() {
        // A wall at z = 0 covering the screen centre; the camera at z = 10.
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
        let wall = [
            Vec3::new(-5.0, -5.0, 0.0),
            Vec3::new(5.0, -5.0, 0.0),
            Vec3::new(0.0, 5.0, 0.0),
        ];
        let screen = [Some([pos2(40.0, 60.0), pos2(60.0, 60.0), pos2(50.0, 40.0)])];
        let occ = Occluders::new(vec![wall], &screen, rect);
        let eye = Vec3::new(0.0, 0.0, 10.0);
        let centre = pos2(50.0, 50.0);
        assert!(
            occ.occluded(eye, Vec3::new(0.0, 0.0, -1.0), centre),
            "behind"
        );
        assert!(
            !occ.occluded(eye, Vec3::new(0.0, 0.0, 1.0), centre),
            "in front"
        );
        assert!(!occ.occluded(eye, Vec3::ZERO, centre), "on the wall itself");
        assert!(
            !occ.occluded(eye, wall[0], pos2(40.0, 60.0)),
            "its own corner"
        );
        // A point whose screen cell holds no triangle is never occluded.
        assert!(!occ.occluded(eye, Vec3::new(0.0, 0.0, -1.0), pos2(3.0, 3.0)));
        // Triangles crossing the near plane are tested everywhere.
        let occ = Occluders::new(vec![wall], &[None], rect);
        assert!(occ.occluded(eye, Vec3::new(0.0, 0.0, -1.0), pos2(3.0, 3.0)));
    }
}
