//! The 2D camera of the UV view: UV space (V up) to screen (y down).

use olvin_editor::glam::Vec2;

/// Pan and zoom. `center` is the UV shown at the canvas centre, `zoom` is
/// screen points per UV unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            center: Vec2::splat(0.5),
            zoom: 400.0,
        }
    }
}

pub const MIN_ZOOM: f32 = 8.0;
pub const MAX_ZOOM: f32 = 200_000.0;

impl View {
    /// UV -> screen, around the canvas centre `origin`.
    pub fn to_screen(&self, origin: Vec2, uv: Vec2) -> Vec2 {
        let d = (uv - self.center) * self.zoom;
        origin + Vec2::new(d.x, -d.y)
    }

    /// Screen -> UV, the inverse of [`View::to_screen`].
    pub fn to_uv(&self, origin: Vec2, screen: Vec2) -> Vec2 {
        let d = screen - origin;
        self.center + Vec2::new(d.x, -d.y) / self.zoom
    }

    /// Zooms by `factor` keeping the UV under `screen` in place.
    pub fn zoom_at(&mut self, origin: Vec2, screen: Vec2, factor: f32) {
        let anchor = self.to_uv(origin, screen);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let moved = self.to_uv(origin, screen);
        self.center += anchor - moved;
    }

    /// Drags the view by a screen-space delta (content follows the mouse).
    pub fn pan(&mut self, delta: Vec2) {
        self.center -= Vec2::new(delta.x, -delta.y) / self.zoom;
    }

    /// Fits the UV box `lo..hi` into a canvas of `size` with some padding.
    pub fn frame(&mut self, size: Vec2, lo: Vec2, hi: Vec2) {
        let extent = (hi - lo).max(Vec2::splat(1e-4));
        let fit = (size * 0.85 / extent).min_element();
        if fit.is_finite() && fit > 0.0 {
            self.zoom = fit.clamp(MIN_ZOOM, MAX_ZOOM);
        }
        self.center = (lo + hi) * 0.5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_roundtrip_and_v_up() {
        let v = View {
            center: Vec2::new(0.25, 0.75),
            zoom: 200.0,
        };
        let o = Vec2::new(300.0, 200.0);
        let uv = Vec2::new(0.6, 0.1);
        assert!(v.to_uv(o, v.to_screen(o, uv)).abs_diff_eq(uv, 1e-5));
        // Higher V is higher on screen (smaller y).
        assert!(v.to_screen(o, Vec2::new(0.0, 1.0)).y < v.to_screen(o, Vec2::ZERO).y);
    }

    #[test]
    fn zoom_keeps_cursor_point_and_pan_follows_mouse() {
        let mut v = View::default();
        let o = Vec2::new(100.0, 100.0);
        let cursor = Vec2::new(160.0, 40.0);
        let before = v.to_uv(o, cursor);
        v.zoom_at(o, cursor, 2.5);
        assert!(v.to_uv(o, cursor).abs_diff_eq(before, 1e-5));
        assert_eq!(v.zoom, 1000.0);
        let uv = Vec2::new(0.3, 0.3);
        let s = v.to_screen(o, uv);
        v.pan(Vec2::new(12.0, -7.0));
        assert!(
            v.to_screen(o, uv)
                .abs_diff_eq(s + Vec2::new(12.0, -7.0), 1e-3)
        );
    }

    #[test]
    fn frame_fits_box() {
        let mut v = View::default();
        v.frame(Vec2::new(400.0, 200.0), Vec2::ZERO, Vec2::new(2.0, 1.0));
        assert_eq!(v.center, Vec2::new(1.0, 0.5));
        assert!((v.zoom - 170.0).abs() < 1e-3);
    }
}
