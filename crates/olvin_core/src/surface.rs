use std::fmt::Debug;
use std::sync::Arc;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

/// Anything a renderer can present to: a winit window, an SDL window, a canvas...
pub trait WindowHandleSource: HasWindowHandle + HasDisplayHandle + Debug + Send + Sync {}
impl<T: HasWindowHandle + HasDisplayHandle + Debug + Send + Sync> WindowHandleSource for T {}

/// Contract between the platform layer and the renderer. The window plugin
/// inserts it and keeps `size` current; the renderer only reads it. Neither
/// side depends on the other's crate.
pub struct SurfaceTarget {
    pub handle: Arc<dyn WindowHandleSource>,
    /// Physical size in pixels. May be (0, 0) while minimized.
    pub size: (u32, u32),
    /// Physical pixels per logical pixel (the OS display scale: 1.0, 1.25,
    /// 2.0...). Screen-space UI multiplies its sizes by it.
    pub scale_factor: f32,
}

impl SurfaceTarget {
    pub fn new(handle: Arc<dyn WindowHandleSource>, size: (u32, u32)) -> Self {
        Self {
            handle,
            size,
            scale_factor: 1.0,
        }
    }

    pub fn with_scale_factor(mut self, scale_factor: f32) -> Self {
        self.scale_factor = scale_factor;
        self
    }

    pub fn aspect(&self) -> f32 {
        self.size.0.max(1) as f32 / self.size.1.max(1) as f32
    }
}
